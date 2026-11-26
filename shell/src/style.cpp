#include "style.h"

#include "strata_bridge/bridge.h"

#include <QApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QStyle>
#include <QStyleFactory>

namespace
{

rust::String rs(const QString& s)
{
  const QByteArray utf8 = s.toUtf8();
  return rust::String(utf8.data(), std::size_t(utf8.size()));
}

QString qs(const rust::String& s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

}  // namespace

Style::Style(QObject* parent) : QObject(parent)
{
  // Renderers write a temp file and rename it over the sheet, or truncate and write: wait
  // for the burst of events to end before reading.
  m_debounce.setSingleShot(true);
  m_debounce.setInterval(150);
  connect(&m_debounce, &QTimer::timeout, this, &Style::onChanged);
  connect(&m_watcher, &QFileSystemWatcher::fileChanged, this, [this] {
    m_debounce.start();
  });
  connect(&m_watcher, &QFileSystemWatcher::directoryChanged, this, [this] {
    m_debounce.start();
  });
}

void Style::setInstance(const QString& root)
{
  m_instance = root;
}

bool Style::apply(const QString& name)
{
  if (!m_watcher.files().isEmpty()) {
    m_watcher.removePaths(m_watcher.files());
  }
  if (!m_watcher.directories().isEmpty()) {
    m_watcher.removePaths(m_watcher.directories());
  }
  m_file.clear();

  // MOApplication::updateStyle: a Qt style name instead of a sheet
  if (QStyleFactory::keys().contains(name, Qt::CaseInsensitive)) {
    qApp->setStyleSheet(QString());
    QApplication::setStyle(name);
    return true;
  }

  const QString found = qs(strata::find_stylesheet(
      rs(name), rs(m_instance), rs(QCoreApplication::applicationDirPath())));
  if (found.isEmpty()) {
    QStringList dirs;
    for (const auto& dir :
         strata::stylesheet_dirs(rs(m_instance), rs(QCoreApplication::applicationDirPath()))) {
      dirs.push_back(qs(dir));
    }
    qWarning("stylesheet \"%s\" not found in: %s", qUtf8Printable(name),
             qUtf8Printable(dirs.join(", ")));
    return false;
  }

  // MO2's sheets are drawn over Windows' own style. The desktop's style here may be anything
  // (QT_STYLE_OVERRIDE=kvantum makes popups translucent and paints their background itself,
  // which a sheet then hides: dropdowns without a background). Fusion is plain enough for the
  // sheets to look as designed.
  if (QApplication::style()->name().compare("fusion", Qt::CaseInsensitive) != 0) {
    QApplication::setStyle("Fusion");
  }
  m_file = found;
  load();
  watch();
  return true;
}

void Style::load()
{
  QFile file(m_file);
  m_content = file.open(QIODevice::ReadOnly) ? file.readAll() : QByteArray();
  // As MO2 does (MOApplication::updateStyle): "file:///" + path, so that url(...) inside
  // the sheet resolves relative to the sheet. Qt cuts exactly those 8 characters off and
  // opens the rest as a plain path, so the absolute path's own leading slash must follow.
  qApp->setStyleSheet("file:///" + QFileInfo(m_file).absoluteFilePath());
}

void Style::watch()
{
  // The sheet itself (edits in place), its folder (a rename over it ends the file watch),
  // and for a symlink also the target and the target's folder (the renderer replaces the
  // target, not the link).
  QStringList paths{m_file, QFileInfo(m_file).absolutePath()};
  const QString target = QFileInfo(m_file).canonicalFilePath();
  if (!target.isEmpty() && target != QFileInfo(m_file).absoluteFilePath()) {
    paths << target << QFileInfo(target).absolutePath();
  }
  for (const QString& path : paths) {
    if (!m_watcher.files().contains(path) && !m_watcher.directories().contains(path)) {
      m_watcher.addPath(path);
    }
  }
}

void Style::onChanged()
{
  if (m_file.isEmpty() || !QFileInfo(m_file).isFile()) {
    // in the middle of a replace, or deleted: keep the current look, and keep watching the
    // folder for the sheet to come back
    return;
  }
  // a renamed-over file is a new inode: watch it again (and a re-pointed link's new target)
  if (!m_watcher.files().isEmpty()) {
    m_watcher.removePaths(m_watcher.files());
  }
  if (!m_watcher.directories().isEmpty()) {
    m_watcher.removePaths(m_watcher.directories());
  }
  watch();

  // the folder also reports other files: only a sheet whose bytes changed is applied again
  QFile file(m_file);
  const QByteArray content = file.open(QIODevice::ReadOnly) ? file.readAll() : QByteArray();
  if (content == m_content) {
    return;
  }
  load();
  qInfo("stylesheet reloaded: %s", qUtf8Printable(m_file));
  emit reloaded(m_file);
}

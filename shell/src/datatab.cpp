#include "datatab.h"

#include "conflicts/preview.h"

#include <QApplication>
#include <QCheckBox>
#include <QLineEdit>
#include <QDateTime>
#include <QFileIconProvider>
#include <QHeaderView>
#include <QLocale>
#include <QMenu>
#include <QPointer>
#include <QPushButton>
#include <QTreeView>

#include <thread>

namespace
{

constexpr int PathRole        = Qt::UserRole + 1;  // the folder's path below Data
constexpr int PlaceholderRole = Qt::UserRole + 2;  // the "not loaded yet" child
constexpr int FromArchiveRole = Qt::UserRole + 3;  // a file inside an archive (no preview)

QString qs(const rust::String& s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

rust::String rs(const QString& s)
{
  const QByteArray utf8 = s.toUtf8();
  return rust::String(utf8.data(), std::size_t(utf8.size()));
}

// FileTreeModel's columns.
QStringList columns()
{
  return {QObject::tr("Name"), QObject::tr("Mod"), QObject::tr("Type"), QObject::tr("Size"),
          QObject::tr("Date modified")};
}

// What Windows' shell calls the file type; MO2 asks SHGetFileInfo. Generic names here.
QString fileType(const QString& name, bool isDir)
{
  if (isDir) {
    return QObject::tr("File folder");
  }
  const int dot = name.lastIndexOf('.');
  if (dot <= 0) {
    return QObject::tr("File");
  }
  return QObject::tr("%1 File").arg(name.mid(dot + 1).toUpper());
}

}  // namespace

DataTab::DataTab(QTreeView* tree, QPushButton* refresh, int area, QLineEdit* filter,
                 QCheckBox* conflictsOnly, QCheckBox* archives, QObject* parent)
    : QObject(parent), m_tree(tree), m_area(area)
{
  if (filter) {
    connect(filter, &QLineEdit::textChanged, this, [this](const QString& text) {
      m_filter = text;
      refill();
    });
  }
  if (conflictsOnly) {
    connect(conflictsOnly, &QCheckBox::toggled, this, [this](bool on) {
      m_conflictsOnly = on;
      refill();
    });
  }
  if (archives) {
    // the archives' file lists are read the first time the box is ticked (a new scan)
    connect(archives, &QCheckBox::toggled, this, [this](bool on) {
      m_archives = on;
      if (on && !m_archivesRead) {
        this->refresh();
      } else {
        refill();
      }
    });
  }
  m_model.setHorizontalHeaderLabels(columns());
  m_tree->setModel(&m_model);
  m_tree->setUniformRowHeights(true);
  m_tree->setSortingEnabled(false);  // the core's order: folders first, then by name
  connect(m_tree, &QTreeView::expanded, this, &DataTab::onExpanded);
  connect(refresh, &QPushButton::clicked, this, &DataTab::refresh);

  // conflicts area: the file's preview, every mod's copy (MO2's FileTree "Preview").
  m_tree->setContextMenuPolicy(Qt::CustomContextMenu);
  connect(m_tree, &QTreeView::customContextMenuRequested, this, [this](const QPoint& pos) {
    const QModelIndex index = m_tree->indexAt(pos).siblingAtColumn(0);
    const QString path      = filePath(index);
    if (path.isEmpty()) {
      return;
    }
    QMenu menu(m_tree);
    QAction* preview = menu.addAction(tr("Preview"));
    preview->setEnabled(strata::conflicts::canPreview(path));
    if (menu.exec(m_tree->viewport()->mapToGlobal(pos)) == preview) {
      strata::conflicts::previewMergedFile(m_tree, m_root, m_profile, path, false);
    }
  });
  connect(m_tree, &QTreeView::doubleClicked, this, [this](const QModelIndex& index) {
    const QString path = filePath(index.siblingAtColumn(0));
    if (!path.isEmpty() && strata::conflicts::canPreview(path)) {
      strata::conflicts::previewMergedFile(m_tree, m_root, m_profile, path, false);
    }
  });
}

QString DataTab::filePath(const QModelIndex& index) const
{
  const QStandardItem* item = m_model.itemFromIndex(index);
  if (!item || item->data(PathRole).isValid() || item->data(PlaceholderRole).toBool() ||
      item->data(FromArchiveRole).toBool()) {
    return {};  // a folder, a placeholder, a file inside an archive or nothing
  }
  const QStandardItem* parent = item->parent();
  const QString folder        = parent ? parent->data(PathRole).toString() : QString();
  return folder.isEmpty() ? item->text() : folder + "/" + item->text();
}

DataTab::~DataTab() = default;

void DataTab::setSource(const QString& root, const QString& profile)
{
  m_root    = root;
  m_profile = profile;
  m_stale   = true;
  m_view.reset();
  m_archivesRead = false;
  m_model.removeRows(0, m_model.rowCount());
}

void DataTab::activate()
{
  if (m_stale) {
    refresh();
  }
}

void DataTab::refresh()
{
  m_stale = false;
  const int generation = ++m_generation;
  m_model.removeRows(0, m_model.rowCount());
  auto* loading = new QStandardItem(tr("Scanning the mods..."));
  loading->setEditable(false);
  m_model.appendRow(loading);

  // The scan opens its own session: nothing on the UI thread is shared with the worker.
  const std::string root    = m_root.toStdString();
  const std::string profile = m_profile.toStdString();
  const bool archives       = m_archives;
  QPointer<DataTab> self(this);
  std::thread([self, generation, root, profile, archives] {
    std::shared_ptr<rust::Box<strata::DataView>> view;
    QString error;
    try {
      auto session = strata::open_session(root, profile);
      view = std::make_shared<rust::Box<strata::DataView>>(session->build_data_view());
      if (archives) {
        (*view)->load_archives(*session);
      }
    } catch (const std::exception& e) {
      error = QString::fromUtf8(e.what());
    }
    QMetaObject::invokeMethod(
        qApp,
        [self, generation, view, error, archives] {
          if (!self || generation != self->m_generation) {
            return;
          }
          self->m_model.removeRows(0, self->m_model.rowCount());
          if (!view) {
            qWarning("Data tab: %s", qUtf8Printable(error));
            return;
          }
          self->m_view         = view;
          self->m_archivesRead = archives;
          self->fill(self->m_model.invisibleRootItem(), QString());
          qInfo("Data tab: %zu files", (*view)->file_count());
        },
        Qt::QueuedConnection);
  }).detach();
}

void DataTab::fill(QStandardItem* folder, const QString& path)
{
  if (!m_view) {
    return;
  }
  static const QFileIconProvider icons;
  const QIcon dirIcon  = icons.icon(QFileIconProvider::Folder);
  const QIcon fileIcon = icons.icon(QFileIconProvider::File);

  const QByteArray filter = m_filter.toUtf8();
  for (const auto& entry : (*m_view)->query(std::uint8_t(m_area), rs(path),
                                            rust::Str(filter.data(), std::size_t(filter.size())),
                                            m_conflictsOnly, m_archives && m_archivesRead)) {
    const QString name = qs(entry.name);
    auto* nameItem     = new QStandardItem(entry.is_dir ? dirIcon : fileIcon, name);
    auto* origin       = new QStandardItem(qs(entry.origin));
    auto* type         = new QStandardItem(fileType(name, entry.is_dir));
    auto* size         = new QStandardItem(entry.is_dir ? QString()
                                                        : QLocale().formattedDataSize(qint64(entry.size)));
    auto* modified = new QStandardItem(
        entry.modified < 0 ? QString()
                           : QLocale().toString(QDateTime::fromSecsSinceEpoch(entry.modified),
                                                QLocale::ShortFormat));
    size->setTextAlignment(Qt::AlignRight | Qt::AlignVCenter);
    for (auto* item : {nameItem, origin, type, size, modified}) {
      item->setEditable(false);
    }
    if (entry.from_archive && !entry.is_dir) {
      nameItem->setData(true, FromArchiveRole);  // the Mod column says "<mod> (<archive>)"
    }
    if (entry.is_dir) {
      const QString childPath = path.isEmpty() ? name : path + "/" + name;
      nameItem->setData(childPath, PathRole);
      // filled when opened
      auto* placeholder = new QStandardItem;
      placeholder->setData(true, PlaceholderRole);
      nameItem->appendRow(placeholder);
    }
    folder->appendRow({nameItem, origin, type, size, modified});
  }
}

void DataTab::refill()
{
  if (!m_view) {
    return;
  }
  m_model.removeRows(0, m_model.rowCount());
  fill(m_model.invisibleRootItem(), QString());
}

void DataTab::onExpanded(const QModelIndex& index)
{
  QStandardItem* item = m_model.itemFromIndex(index.siblingAtColumn(0));
  if (!item || item->rowCount() != 1 || !item->child(0)->data(PlaceholderRole).toBool()) {
    return;
  }
  item->removeRow(0);
  fill(item, item->data(PathRole).toString());
}

#include "layoutstate.h"

#include <algorithm>

#include <QComboBox>
#include <QDockWidget>
#include <QEvent>
#include <QHeaderView>
#include <QMainWindow>
#include <QSplitter>
#include <QStandardItemModel>
#include <QTabWidget>
#include <QToolBar>

namespace
{

enum Kind : std::uint8_t
{
  Text      = 0,
  ByteArray = 1,
  QtValue   = 2,
  StringList = 3,  // '\n'-separated
};

rust::Str rstr(const QByteArray& utf8)
{
  return rust::Str(utf8.data(), std::size_t(utf8.size()));
}

// widgetNameWithTopLevel (settingsutilities.cpp): "<window>_<widget>".
QString nameOf(const QWidget* widget)
{
  const QWidget* top = widget->window();
  if (top == widget) {
    return widget->objectName();
  }
  QStringList parts;
  if (!top->objectName().isEmpty()) {
    parts << top->objectName();
  }
  if (!widget->objectName().isEmpty()) {
    parts << widget->objectName();
  }
  return parts.isEmpty() ? QStringLiteral("unknown_widget") : parts.join('_');
}

// A header's name is its view's (widgetName(const QHeaderView*)).
QString nameOf(const QHeaderView* header)
{
  return nameOf(header->parentWidget());
}

std::optional<QByteArray> load(const strata::Session& session, const char* section,
                               const QString& key)
{
  const QByteArray k = key.toUtf8();
  auto value         = session.load_setting(section, rstr(k));
  if (!value.found) {
    return std::nullopt;
  }
  return QByteArray(reinterpret_cast<const char*>(value.bytes.data()),
                    qsizetype(value.bytes.size()));
}

std::optional<bool> loadBool(const strata::Session& session, const QString& key)
{
  const auto v = load(session, "Geometry", key);
  if (!v) {
    return std::nullopt;
  }
  const QByteArray t = v->trimmed().toLower();
  return !(t.isEmpty() || t == "0" || t == "false");
}


// The number of sections in a QHeaderView::saveState() blob, -1 when it is not one.
// QHeaderViewPrivate::write: marker, version, orientation, sort order, sort section, sort
// shown (1 byte), visual indices and logical indices (QList<int>), hidden sections
// (QBitArray: bit count + bytes), hidden section sizes (QHash<int, int>), length, then the
// section count.
int headerSectionCount(const QByteArray& state)
{
  auto u32 = [&state](qsizetype at) -> qint64 {
    if (at < 0 || at + 4 > state.size()) {
      return -1;
    }
    const auto* p = reinterpret_cast<const unsigned char*>(state.constData()) + at;
    return (qint64(p[0]) << 24) | (p[1] << 16) | (p[2] << 8) | p[3];
  };
  if (u32(0) != 0xff) {
    return -1;
  }
  qsizetype at = 21;
  for (int list = 0; list < 2; ++list) {
    const qint64 n = u32(at);
    if (n < 0) {
      return -1;
    }
    at += 4 + 4 * n;
  }
  const qint64 bits = u32(at);
  if (bits < 0) {
    return -1;
  }
  at += 4 + (bits + 7) / 8;
  const qint64 hidden = u32(at);
  if (hidden < 0) {
    return -1;
  }
  at += 4 + 8 * hidden + 4;  // the hash, then the length
  const qint64 count = u32(at);
  return count < 0 || count > 100000 ? -1 : int(count);
}

// GeometrySettings' isUserSizedSection: a stretched or fit-to-contents section is also sized
// by the view (on every window resize), which is not a change worth saving.
bool isUserSized(const QHeaderView* header, int logical)
{
  if (header->sectionResizeMode(logical) != QHeaderView::Interactive) {
    return false;
  }
  if (header->stretchLastSection()) {
    for (int v = header->count() - 1; v >= 0; --v) {
      const int l = header->logicalIndex(v);
      if (!header->isSectionHidden(l)) {
        return l != logical;
      }
    }
  }
  return true;
}

// Reads another program's header state through `map` (see LayoutState::addHeader): Qt reads
// the blob into a scratch header with that program's column count, and the sizes, hidden
// flags, order and sort column are copied column by column. False when Qt can't read it.
bool restoreMapped(QHeaderView* header, const QByteArray& state, const std::vector<int>& map)
{
  const int count = int(map.size());
  QStandardItemModel model(0, count);
  QHeaderView scratch(header->orientation());
  scratch.setModel(&model);
  if (!scratch.restoreState(state) || scratch.count() != count) {
    return false;
  }
  std::vector<std::pair<int, int>> order;  // (that program's visual index, our column)
  for (int f = 0; f < count; ++f) {
    const int ours = map[f];
    if (ours < 0 || ours >= header->count()) {
      continue;
    }
    header->setSectionHidden(ours, scratch.isSectionHidden(f));
    if (!scratch.isSectionHidden(f)) {
      header->resizeSection(ours, scratch.sectionSize(f));
    }
    order.emplace_back(scratch.visualIndex(f), ours);
  }
  std::sort(order.begin(), order.end());
  for (int v = 0; v < int(order.size()); ++v) {
    header->moveSection(header->visualIndex(order[v].second), v);
  }
  const int sorted = scratch.sortIndicatorSection();
  if (sorted >= 0 && sorted < count && map[sorted] >= 0) {
    header->setSortIndicator(map[sorted], scratch.sortIndicatorOrder());
  }
  return true;
}

}  // namespace

LayoutState::LayoutState(QMainWindow* window) : QObject(window), m_window(window)
{
  m_timer.setSingleShot(true);
  m_timer.setInterval(500);
  connect(&m_timer, &QTimer::timeout, this, &LayoutState::flush);
}

void LayoutState::addHeader(QHeaderView* header)
{
  m_headers.emplace_back(header);
}

void LayoutState::addHeader(QHeaderView* header, std::vector<std::vector<int>> foreignMaps)
{
  m_headers.emplace_back(header);
  m_foreign.insert(header, std::move(foreignMaps));
}

void LayoutState::addSplitter(QSplitter* splitter)
{
  m_splitters.emplace_back(splitter);
}

void LayoutState::addVisibility(QWidget* widget)
{
  m_visibility.emplace_back(widget);
}

void LayoutState::addIndex(QTabWidget* tabs)
{
  m_tabs.emplace_back(tabs);
}

void LayoutState::addIndex(QComboBox* combo)
{
  m_combos.emplace_back(combo);
}

void LayoutState::addList(const QString& key, std::function<std::optional<QStringList>()> get)
{
  m_lists.emplace_back(key, std::move(get));
}

void LayoutState::restore(const strata::Session& session)
{
  m_session = &session;

  if (const auto v = load(session, "Geometry", nameOf(m_window) + "_geometry")) {
    m_window->restoreGeometry(*v);
  } else {
    m_window->resize(1300, 800);  // MainWindow::readSettings
  }
  if (const auto v = load(session, "Geometry", nameOf(m_window) + "_state")) {
    m_window->restoreState(*v);
  }

  // restoreToolbars: one size and button style for every toolbar
  const auto size  = load(session, "Geometry", "toolbar_size");
  const auto style = load(session, "Geometry", "toolbar_button_style");
  for (auto* toolbar : m_window->findChildren<QToolBar*>()) {
    if (size) {
      // "@Size(32 32)"
      const QByteArray inner = size->mid(6, size->size() - 7);
      const auto parts       = inner.split(' ');
      if (size->startsWith("@Size(") && parts.size() == 2) {
        toolbar->setIconSize(QSize(parts[0].toInt(), parts[1].toInt()));
      }
    }
    if (style) {
      toolbar->setToolButtonStyle(Qt::ToolButtonStyle(style->toInt()));
    }
    if (const auto visible = loadBool(session, nameOf(toolbar) + "_visibility")) {
      toolbar->setVisible(*visible);
    }
  }

  for (auto& splitter : m_splitters) {
    if (const auto v = load(session, "Geometry", nameOf(splitter) + "_state")) {
      splitter->restoreState(*v);
    }
  }
  for (auto& widget : m_visibility) {
    if (const auto visible = loadBool(session, nameOf(widget) + "_visibility")) {
      widget->setVisible(*visible);
    }
  }
  for (auto& tabs : m_tabs) {
    if (const auto v = load(session, "Widgets", nameOf(tabs) + "_index")) {
      tabs->setCurrentIndex(v->toInt());
    }
  }
  for (auto& combo : m_combos) {
    if (const auto v = load(session, "Widgets", nameOf(combo) + "_index")) {
      combo->setCurrentIndex(v->toInt());
    }
  }
  for (auto& header : m_headers) {
    if (const auto v = load(session, "Geometry", nameOf(header) + "_state")) {
      const int sections = headerSectionCount(*v);
      const auto foreign = m_foreign.value(header.data());
      const auto map = std::find_if(foreign.begin(), foreign.end(),
                                    [sections](const auto& m) { return int(m.size()) == sections; });
      if (sections == header->count()) {
        header->restoreState(*v);
        m_restored.insert(header.data());
      } else if (map != foreign.end() && restoreMapped(header.data(), *v, *map)) {
        qInfo("%s: the saved header has %d columns (another program's), read column by column",
              qUtf8Printable(nameOf(header.data())), sections);
        m_restored.insert(header.data());
      } else {
        // a header this list can't read: keep the defaults and leave the value alone
        qInfo("%s: the saved header has %d columns, this list %d: not restored",
              qUtf8Printable(nameOf(header.data())), sections, header->count());
      }
    }
  }

  if (m_connected) {
    return;
  }
  m_connected = true;
  // The first show, layout and resize events are the restore itself, not changes: arm a
  // second after the start.
  QTimer::singleShot(1000, this, [this] {
    for (const auto& value : collect()) {
      if (m_listsTouched && value.kind == StringList) {
        continue;  // the user changed it already
      }
      m_baseline[QString::fromLatin1(value.section) + "/" + value.key] = value.bytes;
    }
    m_armed = true;
    if (m_listsTouched) {
      m_timer.start();
    }
  });
  m_window->installEventFilter(this);
  for (auto* toolbar : m_window->findChildren<QToolBar*>()) {
    toolbar->installEventFilter(this);
    connect(toolbar, &QToolBar::iconSizeChanged, this, &LayoutState::changed);
    connect(toolbar, &QToolBar::toolButtonStyleChanged, this, &LayoutState::changed);
  }
  for (auto* dock : m_window->findChildren<QDockWidget*>()) {
    dock->installEventFilter(this);
  }
  for (auto& header : m_headers) {
    QHeaderView* h = header;
    connect(h, &QHeaderView::sectionResized, this, [this, h](int logical, int oldSize, int newSize) {
      // hiding or showing a section is a resize to or from 0
      if (oldSize != 0 && newSize != 0 && !isUserSized(h, logical)) {
        return;
      }
      headerChanged(h);
    });
    connect(h, &QHeaderView::sectionMoved, this, [this, h] { headerChanged(h); });
    connect(h, &QHeaderView::sortIndicatorChanged, this, [this, h] { headerChanged(h); });
  }
  for (auto& splitter : m_splitters) {
    connect(splitter, &QSplitter::splitterMoved, this, &LayoutState::changed);
  }
  for (auto& widget : m_visibility) {
    widget->installEventFilter(this);
  }
  for (auto& tabs : m_tabs) {
    connect(tabs, &QTabWidget::currentChanged, this, &LayoutState::changed);
  }
  for (auto& combo : m_combos) {
    connect(combo, &QComboBox::currentIndexChanged, this, &LayoutState::changed);
  }
}

bool LayoutState::eventFilter(QObject* watched, QEvent* event)
{
  switch (event->type()) {
  case QEvent::Move:
  case QEvent::Resize:
  case QEvent::Show:
  case QEvent::Hide:
  case QEvent::WindowStateChange:
    changed();
    break;
  default:
    break;
  }
  return QObject::eventFilter(watched, event);
}

void LayoutState::headerChanged(QHeaderView* header)
{
  if (m_armed && m_quiet == 0) {
    m_dirtyHeaders.insert(header);
    m_timer.start();
  }
}

void LayoutState::changed()
{
  if (m_armed && m_quiet == 0) {
    m_timer.start();
  }
}

LayoutState::Values LayoutState::collect() const
{
  Values v;
  // MainWindow::storeSettings, in its order
  v.push_back({"Geometry", nameOf(m_window) + "_state", m_window->saveState(), ByteArray});
  v.push_back({"Geometry", nameOf(m_window) + "_geometry", m_window->saveGeometry(), ByteArray});
  for (auto* dock : m_window->findChildren<QDockWidget*>()) {
    // saveDocks: the width of docks beside the centre, the height of the others
    const auto area = m_window->dockWidgetArea(dock);
    const int size  = (area == Qt::LeftDockWidgetArea || area == Qt::RightDockWidgetArea)
                          ? dock->width()
                          : dock->height();
    v.push_back({"Geometry", "MainWindow_docks_" + dock->objectName() + "_size",
                 QByteArray::number(size), Text});
  }
  const auto toolbars = m_window->findChildren<QToolBar*>();
  for (auto* toolbar : toolbars) {
    v.push_back({"Geometry", nameOf(toolbar) + "_visibility",
                 toolbar->isVisibleTo(m_window) ? "true" : "false", Text});
  }
  if (!toolbars.isEmpty()) {
    const QSize s = toolbars.first()->iconSize();
    v.push_back({"Geometry", "toolbar_size",
                 QStringLiteral("@Size(%1 %2)").arg(s.width()).arg(s.height()).toUtf8(), QtValue});
    v.push_back({"Geometry", "toolbar_button_style",
                 QByteArray::number(int(toolbars.first()->toolButtonStyle())), Text});
  }
  for (const auto& splitter : m_splitters) {
    if (splitter) {
      v.push_back({"Geometry", nameOf(splitter) + "_state", splitter->saveState(), ByteArray});
    }
  }
  for (const auto& widget : m_visibility) {
    if (widget) {
      v.push_back({"Geometry", nameOf(widget) + "_visibility",
                   widget->isVisibleTo(m_window) ? "true" : "false", Text});
    }
  }
  for (const auto& header : m_headers) {
    // a header is only saved after the user changed it (GeometrySettings::autoSaveState):
    // the view also changes its bytes by itself (its length follows the window)
    if (header && m_dirtyHeaders.contains(header.data())) {
      v.push_back({"Geometry", nameOf(header.data()) + "_state", header->saveState(), ByteArray});
    }
  }
  for (const auto& tabs : m_tabs) {
    if (tabs) {
      v.push_back({"Widgets", nameOf(tabs) + "_index", QByteArray::number(tabs->currentIndex()), Text});
    }
  }
  for (const auto& combo : m_combos) {
    if (combo) {
      v.push_back(
          {"Widgets", nameOf(combo) + "_index", QByteArray::number(combo->currentIndex()), Text});
    }
  }
  for (const auto& [key, get] : m_lists) {
    if (const auto items = get()) {
      v.push_back({"Widgets", key, items->join('\n').toUtf8(), StringList});
    }
  }
  return v;
}

void LayoutState::flush()
{
  m_timer.stop();
  if (!m_session || !m_armed) {
    return;
  }
  // only what differs from the restored layout (or the last write): a value the user never
  // touched stays as the ini has it, even when Qt would write the same layout in other bytes
  // (or another program's layout this window cannot restore, such as Fluorine's 13 mod list
  // columns)
  rust::Vec<strata::SettingValue> changed;
  for (const auto& value : collect()) {
    const QString id = QString::fromLatin1(value.section) + "/" + value.key;
    auto it          = m_baseline.find(id);
    if (it != m_baseline.end() && *it == value.bytes) {
      continue;
    }
    m_baseline[id] = value.bytes;
    strata::SettingValue v;
    v.section          = value.section;
    const QByteArray k = value.key.toUtf8();
    v.key              = rust::String(k.data(), std::size_t(k.size()));
    for (char c : value.bytes) {
      v.bytes.push_back(std::uint8_t(c));
    }
    v.kind = value.kind;
    changed.push_back(std::move(v));
  }
  if (changed.empty()) {
    return;
  }
  try {
    if (m_session->store_settings(std::move(changed))) {
      qDebug("layout saved to ModOrganizer.ini");
    }
  } catch (const std::exception& e) {
    qWarning("cannot save the window layout: %s", e.what());
  }
}

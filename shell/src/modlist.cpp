#include "modlist.h"

#include <QApplication>
#include <QDateTime>
#include <QHeaderView>
#include <QIcon>
#include <QMimeData>
#include <QUrl>
#include <QPainter>
#include <QTreeView>

#include <algorithm>
#include <climits>

namespace
{

QString qs(const rust::String& s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

QColor rgba(std::uint32_t c)
{
  return QColor(int((c >> 24) & 0xff), int((c >> 16) & 0xff), int((c >> 8) & 0xff), int(c & 0xff));
}

// ColorSettings::idealTextColor.
QColor idealTextColor(const QColor& background)
{
  if (background.alpha() < 50) {
    return QColor(Qt::black);
  }
  const int luminance = int(background.red() * 0.299 + background.green() * 0.587 +
                            background.blue() * 0.114);
  return QColor(luminance >= 128 ? Qt::black : Qt::white);
}

QList<QStandardItem*> makeRow(const strata::ModRow& row, const QString& gameName)
{
  using namespace modlist;
  const auto kind = Kind(row.kind);

  QList<QStandardItem*> items;
  for (int c = 0; c < ColumnCount; ++c) {
    auto* item = new QStandardItem;
    item->setEditable(false);
    // ModList::flags: everything but backups and Overwrite can be dragged; only
    // separators take drops onto them
    item->setDragEnabled(kind != Backup && kind != Overwrite);
    item->setDropEnabled(kind == Separator);
    item->setData(int(kind), KindRole);
    // backups first and Overwrite last by ascending priority (ModListByPriorityProxy::buildTree)
    const int priority = kind == Backup      ? -1
                         : kind == Overwrite ? INT_MAX
                                             : int(row.priority);
    item->setData(priority, PriorityRole);
    item->setData(qs(row.name), InternalNameRole);
    item->setData(qint64(row.nexus_id), NexusIdRole);
    item->setData(qs(row.comments), CommentsRole);
    item->setData(qs(row.category), CategoryRole);
    item->setData(row.nest, NestRole);
    item->setData(row.stored_size, StoredSizeRole);
    // ModList::data, Qt::TextAlignmentRole
    item->setTextAlignment(c == ColVersion ? (Qt::AlignRight | Qt::AlignVCenter)
                           : c == ColNotes ? (Qt::AlignLeft | Qt::AlignVCenter)
                                           : Qt::AlignCenter);
    items.push_back(item);
  }

  auto* name = items[ColName];
  name->setText(qs(row.display_name));
  const bool centered = kind == Separator || kind == Overwrite;  // HIGHLIGHT_CENTER
  name->setTextAlignment(centered ? Qt::AlignCenter : (Qt::AlignLeft | Qt::AlignVCenter));
  if (row.checkable) {
    // a click goes through the core (MainWindow, itemChanged); the model follows its events
    name->setCheckable(true);
    name->setData(row.enabled ? Qt::Checked : Qt::Unchecked, Qt::CheckStateRole);
  }
  QFont font;
  if (kind == Separator) {
    font.setItalic(true);
    font.setBold(true);
  } else if (kind == Overwrite && !row.has_files) {
    font.setItalic(true);  // HIGHLIGHT_INVALID
  }
  name->setFont(font);
  if (kind == Overwrite) {
    name->setForeground(row.has_files ? QBrush(Qt::darkRed) : QBrush(Qt::darkGray));
  }

  if (row.has_color && (kind == Separator)) {
    const QColor color = rgba(row.color);
    for (auto* item : items) {
      item->setBackground(color);
    }
  } else if (row.has_color) {
    items[ColNotes]->setBackground(rgba(row.color));
  }

  // Flags (ModFlagIconDelegate) and Content (ModContentIconDelegate)
  QStringList flagIcons, flagTexts;
  for (const auto& i : row.flag_icons) {
    flagIcons << qs(i);
  }
  for (const auto& t : row.flag_texts) {
    flagTexts << (qs(t) == "Notes" ? qs(row.notes) : qs(t));
  }
  items[ColFlags]->setData(flagIcons, BaseFlagIconsRole);
  items[ColFlags]->setData(flagTexts, BaseFlagTextsRole);
  items[ColFlags]->setData(flagIcons, IconsRole);
  items[ColFlags]->setToolTip(flagTexts.join("<br>"));
  QStringList contentIcons, contentNames;
  QList<int> contentIds;
  for (std::size_t i = 0; i < row.contents.size(); ++i) {
    contentIds << int(row.contents[i]);
    if (!row.content_icons[i].empty()) {
      contentIcons << qs(row.content_icons[i]);
    }
    contentNames << qs(row.content_names[i]);
  }
  items[ColContent]->setData(contentIcons, IconsRole);
  items[ColContent]->setData(QVariant::fromValue(contentIds), ContentIdsRole);
  items[ColName]->setData(QVariant::fromValue(contentIds), ContentIdsRole);
  items[ColContent]->setToolTip(contentNames.join("<br>"));

  QList<int> categories;
  for (int id : row.categories) {
    categories << id;
  }
  name->setData(QVariant::fromValue(categories), CategoriesRole);
  name->setData(row.endorsed, EndorsedRole);
  name->setData(row.tracked, TrackedRole);
  name->setData(row.valid, ValidRole);

  items[ColCategory]->setText(qs(row.category));
  if (kind == Foreign) {
    QFont italic;
    italic.setItalic(true);
    items[ColCategory]->setFont(italic);
  }
  if (row.nexus_id > 0) {
    items[ColModId]->setData(qint64(row.nexus_id), Qt::DisplayRole);
  }
  if (kind != Separator && kind != Overwrite) {
    items[ColGame]->setText(gameName);
  }
  items[ColVersion]->setText(qs(row.version));
  items[ColName]->setData(int(row.update), UpdateRole);
  if (row.update != 0) {
    // ModList::data for COL_VERSION: bold and red with an icon when the newest version
    // differs, dark green when it is installed; the tooltip names both
    QStandardItem* v = items[ColVersion];
    const bool differs = row.update == 2 || row.update == 3;
    v->setForeground(differs ? QBrush(Qt::red) : QBrush(Qt::darkGreen));
    if (differs) {
      QFont bold;
      bold.setBold(true);
      v->setFont(bold);
      v->setIcon(QIcon(row.update == 2 ? ":/MO/gui/update_available" : ":/MO/gui/warning"));
    }
    QString tip = QObject::tr("installed version: \"%1\", newest version: \"%2\"")
                      .arg(qs(row.version), qs(row.newest_version));
    if (row.update == 3) {
      tip += "<br>" + QObject::tr("The newest version on Nexus seems to be older than the one you have installed. "
                                  "This could either mean the version you have has been withdrawn (i.e. due to a "
                                  "bug) or the author uses a non-standard versioning scheme and that newest "
                                  "version is actually newer. Either way you may want to \"upgrade\".");
    }
    v->setToolTip(tip);
  }
  if (row.install_time >= 0) {
    items[ColInstallTime]->setData(QDateTime::fromSecsSinceEpoch(row.install_time),
                                   Qt::DisplayRole);
  }
  if (row.priority >= 0) {
    items[ColPriority]->setText(QString::number(row.priority));
  }
  items[ColNotes]->setText(qs(row.comments));
  return items;
}

}  // namespace

namespace modlist
{

QStringList columnNames()
{
  return {QObject::tr("Mod Name"), QObject::tr("Conflicts"), QObject::tr("Flags"),
          QObject::tr("Content"),  QObject::tr("Category"),  QObject::tr("Nexus ID"),
          QObject::tr("Source Game"), QObject::tr("Version"), QObject::tr("Installation"),
          QObject::tr("Priority"), QObject::tr("Notes")};
}

void fill(QStandardItemModel& model, const rust::Vec<strata::ModRow>& rows, bool nested,
          const QString& gameName, QHash<QString, QStandardItem*>* byName)
{
  // rows only: clearing the columns too would reset the header (hidden columns, sizes)
  model.removeRows(0, model.rowCount());
  if (model.columnCount() != ColumnCount) {
    model.setColumnCount(ColumnCount);
    model.setHorizontalHeaderLabels(columnNames());
  }

  std::vector<QStandardItem*> firstItems;
  firstItems.reserve(rows.size());
  for (const auto& row : rows) {
    auto items = makeRow(row, gameName);
    firstItems.push_back(items[ColName]);
    if (byName) {
      byName->insert(qs(row.name), items[ColName]);
    }
    QStandardItem* parent = nullptr;
    if (nested && row.parent >= 0 && std::size_t(row.parent) < firstItems.size()) {
      parent = firstItems[std::size_t(row.parent)];
    }
    if (parent) {
      parent->appendRow(items);
    } else {
      model.appendRow(items);
    }
  }
}

void applyConflicts(QStandardItem* nameItem, const strata::ConflictRow& row)
{
  QStandardItemModel* model = nameItem->model();
  const QModelIndex index   = nameItem->index();
  QStandardItem* conflicts  = model->itemFromIndex(index.siblingAtColumn(ColConflictFlags));
  QStandardItem* flags      = model->itemFromIndex(index.siblingAtColumn(ColFlags));
  if (!conflicts || !flags) {
    return;
  }
  QStringList icons, texts;
  for (const auto& i : row.icons) {
    icons << qs(i);
  }
  for (const auto& t : row.texts) {
    texts << qs(t);
  }
  if (std::all_of(icons.begin(), icons.end(), [](const QString& i) { return i.isEmpty(); })) {
    icons.clear();
  }
  conflicts->setData(icons, IconsRole);
  conflicts->setToolTip(texts.join("<br>"));
  // ModInfoWithConflictInfo::getFlags puts "hidden files" first
  QStringList flagIcons = flags->data(BaseFlagIconsRole).toStringList();
  QStringList flagTexts = flags->data(BaseFlagTextsRole).toStringList();
  if (row.hidden_files) {
    flagIcons.prepend(":/MO/gui/emblem_hidden_files");
    flagTexts.prepend(QObject::tr("Contains hidden files"));
  }
  flags->setData(flagIcons, IconsRole);
  flags->setToolTip(flagTexts.join("<br>"));
}

void update(QStandardItem* nameItem, const strata::ModRow& row, const QString& gameName)
{
  QStandardItemModel* model = nameItem->model();
  const QModelIndex index   = nameItem->index();
  const auto fresh          = makeRow(row, gameName);
  for (int c = 0; c < ColumnCount; ++c) {
    QStandardItem* target = c == 0 ? nameItem : model->itemFromIndex(index.siblingAtColumn(c));
    if (target) {
      for (int role : {int(Qt::DisplayRole), int(Qt::CheckStateRole), int(Qt::BackgroundRole),
                       int(Qt::ForegroundRole), int(Qt::FontRole), int(Qt::ToolTipRole),
                       int(CommentsRole), int(CategoryRole), int(NexusIdRole), int(NestRole),
                       int(StoredSizeRole), int(BaseFlagIconsRole), int(BaseFlagTextsRole),
                       int(ContentIdsRole), int(CategoriesRole), int(EndorsedRole),
                       int(TrackedRole), int(ValidRole), int(UpdateRole), int(Qt::DecorationRole)}) {
        target->setData(fresh[c]->data(role), role);
      }
      if (c != ColConflictFlags) {
        target->setData(fresh[c]->data(IconsRole), IconsRole);
      }
    }
  }
  qDeleteAll(fresh);
}

int visibleActiveCount(const QAbstractItemModel& model)
{
  int count = 0;
  std::vector<QModelIndex> stack{QModelIndex()};
  while (!stack.empty()) {
    const QModelIndex parent = stack.back();
    stack.pop_back();
    for (int r = 0; r < model.rowCount(parent); ++r) {
      const QModelIndex index = model.index(r, ColName, parent);
      if (index.data(KindRole).toInt() == Regular &&
          index.data(Qt::CheckStateRole).toInt() == Qt::Checked) {
        ++count;
      }
      if (model.hasChildren(index)) {
        stack.push_back(index);
      }
    }
  }
  return count;
}

}  // namespace modlist

ModListProxy::ModListProxy(QObject* parent) : QSortFilterProxyModel(parent)
{
  std::fill(std::begin(m_visible), std::end(m_visible), true);
  // a separator stays while something inside it matches
  setRecursiveFilteringEnabled(true);
  setSortRole(Qt::DisplayRole);
}

QStringList ModListProxy::mimeTypes() const
{
  return QSortFilterProxyModel::mimeTypes() << QStringLiteral("text/uri-list") << QStringLiteral("text/plain");
}

QMimeData* ModListProxy::mimeData(const QModelIndexList& indexes) const
{
  QMimeData* data = QSortFilterProxyModel::mimeData(indexes);
  if (data) {
    data->setText(QStringLiteral("mod"));  // ModList::mimeData
  }
  return data;
}

bool ModListProxy::canDropMimeData(const QMimeData* data, Qt::DropAction action, int row,
                                   int column, const QModelIndex& parent) const
{
  // ModListDropInfo: a drop is valid when it holds rows of the list or of the downloads, or
  // local URLs; whether an URL is an archive or a folder is decided at the drop
  if (data->hasUrls()) {
    const auto urls = data->urls();
    return std::all_of(urls.begin(), urls.end(), [](const QUrl& u) { return u.isLocalFile(); });
  }
  if (data->text() == "mod" || data->text() == "download") {
    return true;
  }
  return QSortFilterProxyModel::canDropMimeData(data, action, row, column, parent);
}

template <class Change>
void ModListProxy::refilter(Change change)
{
#if QT_VERSION >= QT_VERSION_CHECK(6, 10, 0)
  beginFilterChange();
  change();
  endFilterChange(QSortFilterProxyModel::Direction::Rows);
#else
  change();
  invalidateRowsFilter();
#endif
}

void ModListProxy::setFilterText(const QString& text)
{
  refilter([&] { m_filter = text; });
}

void ModListProxy::setDisplayForeign(bool display)
{
  refilter([&] { m_displayForeign = display; });
}

void ModListProxy::setCriteriaPass(std::optional<QSet<QString>> names)
{
  refilter([&] { m_criteriaPass = std::move(names); });
}

void ModListProxy::setColumnVisible(int column, bool visible)
{
  if (column < 0 || column >= modlist::ColumnCount || m_visible[column] == visible) {
    return;
  }
  refilter([&] { m_visible[column] = visible; });
}

bool ModListProxy::filterAcceptsRow(int row, const QModelIndex& parent) const
{
  using namespace modlist;
  const QModelIndex index = sourceModel()->index(row, ColName, parent);
  if (!m_displayForeign && index.data(KindRole).toInt() == Foreign) {
    return false;
  }
  if (m_criteriaPass && !m_criteriaPass->contains(index.data(InternalNameRole).toString())) {
    return false;
  }
  if (m_filter.isEmpty()) {
    return true;
  }
  const QByteArray filter   = m_filter.toUtf8();
  const QByteArray name     = index.data(InternalNameRole).toString().toUtf8();
  const QByteArray comments = m_visible[ColNotes] ? index.data(CommentsRole).toString().toUtf8()
                                                  : QByteArray();
  const QByteArray category = m_visible[ColCategory]
                                  ? index.data(CategoryRole).toString().toUtf8()
                                  : QByteArray();
  const qint64 id = m_visible[ColModId] ? index.data(NexusIdRole).toLongLong() : -1;
  return strata::mod_matches_filter(rust::Str(filter.data(), std::size_t(filter.size())),
                                    rust::Str(name.data(), std::size_t(name.size())),
                                    rust::Str(comments.data(), std::size_t(comments.size())),
                                    rust::Str(category.data(), std::size_t(category.size())),
                                    id);
}

bool ModListProxy::lessThan(const QModelIndex& left, const QModelIndex& right) const
{
  if (left.column() == modlist::ColPriority) {
    return left.data(modlist::PriorityRole).toInt() < right.data(modlist::PriorityRole).toInt();
  }
  return QSortFilterProxyModel::lessThan(left, right);
}

bool modlist::isCollapsedSeparator(const QTreeView* view, const QModelIndex& index)
{
  const QModelIndex row = index.siblingAtColumn(0);
  return row.data(KindRole).toInt() == Separator && row.model()->hasChildren(row) && !view->isExpanded(row);
}

void modlist::forEachDescendant(const QModelIndex& separator, const std::function<void(const QModelIndex&)>& f)
{
  const QModelIndex parent = separator.siblingAtColumn(0);
  const auto* model        = parent.model();
  for (int r = 0; r < model->rowCount(parent); ++r) {
    const QModelIndex child = model->index(r, 0, parent);
    f(child);
    if (model->hasChildren(child)) {
      forEachDescendant(child, f);
    }
  }
}

ModListDelegate::ModListDelegate(QTreeView* view, bool alwaysShowSize,
                                 const QHash<QString, QColor>* markers, const modlist::RollUps* rollUps)
    : QStyledItemDelegate(view), m_view(view), m_alwaysShowSize(alwaysShowSize), m_markers(markers),
      m_rollUps(rollUps)
{}

IconDelegate::IconDelegate(QTreeView* view, int column, int compactSize, const modlist::RollUps* rollUps)
    : QStyledItemDelegate(view), m_view(view), m_column(column), m_compactSize(compactSize), m_rollUps(rollUps)
{}

// The row's icons; a collapsed separator's combined with every row's inside it (MO2 combines
// them as a set: each icon once, compact)
QStringList IconDelegate::icons(const QModelIndex& index) const
{
  QStringList own = index.data(modlist::IconsRole).toStringList();
  const bool rollUp = m_rollUps && modlist::isCollapsedSeparator(m_view, index) &&
                      ((m_column == modlist::ColFlags && m_rollUps->flags) ||
                       (m_column == modlist::ColConflictFlags && m_rollUps->conflicts) ||
                       (m_column == modlist::ColContent && m_rollUps->contents));
  if (!rollUp) {
    return own;
  }
  own.removeAll(QString());
  modlist::forEachDescendant(index, [&](const QModelIndex& child) {
    for (const QString& icon : child.siblingAtColumn(m_column).data(modlist::IconsRole).toStringList()) {
      if (!icon.isEmpty() && !own.contains(icon)) {
        own << icon;
      }
    }
  });
  return own;
}

void IconDelegate::paint(QPainter* painter, const QStyleOptionViewItem& option,
                         const QModelIndex& index) const
{
  // the row's background (selection, separator colour) first, as any item
  QStyleOptionViewItem opt(option);
  initStyleOption(&opt, index);
  opt.text.clear();
  QStyle* style = opt.widget ? opt.widget->style() : QApplication::style();
  // a coloured cell (a separator) is drawn without the widget, so a sheet's `::item` rule
  // can't drop the colour (see ModListDelegate::paint)
  const bool coloured =
      index.data(Qt::BackgroundRole).value<QColor>().isValid() && !(option.state & QStyle::State_Selected);
  style->drawControl(QStyle::CE_ItemViewItem, &opt, painter, coloured ? nullptr : opt.widget);

  QStringList icons = this->icons(index);
  if (m_view->header()->sectionSize(m_column) < m_compactSize) {
    icons.removeAll(QString());
  }
  if (icons.isEmpty()) {
    return;
  }
  // IconDelegate::paintIcons
  int iconWidth = std::min(16, (option.rect.width() / int(icons.size())) - 4);
  if (iconWidth <= 0) {
    return;
  }
  const int margin = (option.rect.height() - iconWidth) / 2;
  int x            = 4;
  painter->save();
  painter->translate(option.rect.topLeft());
  for (const QString& icon : icons) {
    if (!icon.isEmpty()) {
      painter->drawPixmap(x, margin, QIcon(icon).pixmap(iconWidth, iconWidth));
    }
    x += iconWidth + 4;
  }
  painter->restore();
}

QSize IconDelegate::sizeHint(const QStyleOptionViewItem& option, const QModelIndex& index) const
{
  const auto n = icons(index).size();
  QSize size   = QStyledItemDelegate::sizeHint(option, index);
  size.setWidth(int(n) * 20);
  return size;
}

void ModListDelegate::initStyleOption(QStyleOptionViewItem* option,
                                      const QModelIndex& index) const
{
  // the base class always overwrites the background brush: keep the one paint() set
  const QColor background = option->backgroundBrush.color();
  QStyledItemDelegate::initStyleOption(option, index);
  if (background.isValid()) {
    option->backgroundBrush = background;
  }
}

void ModListDelegate::paint(QPainter* painter, const QStyleOptionViewItem& option,
                            const QModelIndex& index) const
{
  using namespace modlist;
  QStyleOptionViewItem opt(option);
  const bool separator = index.data(KindRole).toInt() == Separator;
  const bool nameColumn = index.column() == ColName;

  if (nameColumn) {
    if (m_view->indentation() == 0) {
      // no indentation: room for the separators' branch arrow, which Qt doesn't draw then
      const int branchSize = option.rect.height();
      opt.rect.adjust(branchSize, 0, 0, 0);
      if (separator) {
        // the row's palette, font and direction (a bare option has the application's palette:
        // an arrow drawn in its colours vanished on dark sheets such as riki)
        QStyleOptionViewItem branch;
        branch.QStyleOption::operator=(option);
        branch.rect  = QRect(option.rect.x(), option.rect.y(), branchSize, branchSize);
        branch.state = option.state | QStyle::State_Item;
        if (index.model()->hasChildren(index)) {
          branch.state |= QStyle::State_Children;
        }
        if (m_view->isExpanded(index)) {
          branch.state |= QStyle::State_Open;
        }
        QStyle* style = option.widget ? option.widget->style() : QApplication::style();
        style->drawPrimitive(QStyle::PE_IndicatorBranch, &branch, painter, option.widget);
      }
    } else if (index.parent().isValid() && !separator) {
      // mods line up with their separator; separators keep their indentation
      opt.rect.adjust(-m_view->indentation(), 0, 0, 0);
    }
  }

  // the selection's conflict highlight wins over the row's own colour (markerColor)
  QColor color = m_markers ? m_markers->value(index.data(InternalNameRole).toString()) : QColor();
  // a collapsed separator: the average of the highlights inside (mods only: a nested
  // separator's colour comes from its own mods), as markerColor
  if (!color.isValid() && m_markers && !m_markers->isEmpty() && m_rollUps && m_rollUps->highlightTo &&
      isCollapsedSeparator(m_view, index)) {
    int r = 0, g = 0, b = 0, a = 0, n = 0;
    forEachDescendant(index, [&](const QModelIndex& child) {
      if (child.data(KindRole).toInt() == Separator) {
        return;
      }
      const QColor c = m_markers->value(child.data(InternalNameRole).toString());
      if (c.isValid()) {
        r += c.red();
        g += c.green();
        b += c.blue();
        a += c.alpha();
        ++n;
      }
    });
    if (n > 0) {
      color = QColor(r / n, g / n, b / n, a / n);
    }
  }
  if (!color.isValid()) {
    color = index.data(Qt::BackgroundRole).value<QColor>();
  }
  opt.backgroundBrush = color;
  if (color.isValid() && ((separator && (nameColumn || index.column() == ColPriority)) ||
                          index.column() == ColNotes)) {
    // the colour over the row's background, then the readable text colour for it
    QColor base = opt.palette.base().color();
    if (opt.features & QStyleOptionViewItem::Alternate) {
      base = opt.palette.alternateBase().color();
    }
    const double a = color.alpha() / 255.;
    const QColor mixed(int((1 - a) * base.red() + a * color.red()),
                       int((1 - a) * base.green() + a * color.green()),
                       int((1 - a) * base.blue() + a * color.blue()));
    opt.palette.setBrush(QPalette::Text, idealTextColor(mixed));
  }

  // A stylesheet with an `::item` rule that has a box (riki: `QAbstractItemView::item {
  // padding: 2px 0; border: none; }`) or a background makes QStyleSheetStyle draw the item
  // from the rule and drop the row's own colour (backgroundBrush): separators turned black.
  // A coloured cell that is not selected is therefore drawn without the widget, so the sheet's
  // rules don't apply and the base style paints the colour, as MO2's colours look with
  // sheets that have no such rule. A selected cell keeps the sheet's selection look.
  if (color.isValid() && !(option.state & QStyle::State_Selected)) {
    opt.widget = nullptr;
    QStyledItemDelegate::paint(painter, opt, index);
  } else {
    QStyledItemDelegate::paint(painter, opt, index);
  }

  if (separator && nameColumn) {
    // Sub-Separator: one '#' per level, and the number of items inside
    const int level = index.data(NestRole).toInt();
    QString text = level < 7 ? QString("#").repeated(level) : "#" + QString::number(level);
    if (m_alwaysShowSize || index.data(StoredSizeRole).toBool()) {
      if (!text.isEmpty()) {
        text += ' ';
      }
      text += QString("(%1)").arg(index.model()->rowCount(index));
    }
    if (!text.isEmpty()) {
      painter->save();
      painter->setPen(QPen(opt.palette.brush(QPalette::Text).color()));
      painter->drawText(opt.rect.adjusted(5, 0, 5, 0), Qt::AlignVCenter | Qt::AlignLeft, text);
      painter->restore();
    }
  }
}

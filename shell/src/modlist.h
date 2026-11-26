#pragma once

#include <functional>

// The mod list (left pane): MO2's ModList columns and roles, filled from the core's rows,
// with the sort/filter proxy and the delegate that draws separators (modlist.cpp,
// modlistsortproxy.cpp, modlistview.cpp in MO2).

#include "strata_bridge/bridge.h"

#include <QHash>
#include <QSet>

#include <optional>
#include <QSortFilterProxyModel>
#include <QStandardItemModel>
#include <QStyledItemDelegate>

class QTreeView;

namespace modlist
{

// ModList::EColumn, in MO2's order: saved header states depend on it.
enum Column
{
  ColName,
  ColConflictFlags,
  ColFlags,
  ColContent,
  ColCategory,
  ColModId,
  ColGame,
  ColVersion,
  ColInstallTime,
  ColPriority,
  ColNotes,
  ColumnCount
};

enum Role
{
  KindRole = Qt::UserRole + 1,  // Kind
  PriorityRole,                 // int, for sorting; backups first, Overwrite last
  NestRole,                     // separator level
  StoredSizeRole,               // bool: draw "(n)" even when "always show size" is off
  InternalNameRole,             // the folder name (the filter matches it)
  NexusIdRole,                  // qint64, -1 when none
  CommentsRole,
  CategoryRole,
  IconsRole,       // QStringList: the icons of the Flags, Conflicts and Content columns
  ContentIdsRole,  // QList<int>: strata_ui::content::Content ids (the filters)
  BaseFlagIconsRole,  // the Flags column without the conflict index's "hidden files"
  BaseFlagTextsRole,
  CategoriesRole,  // QList<int>: every category id (the filter panel)
  EndorsedRole,
  TrackedRole,
  ValidRole,       // valid game data
  UpdateRole,      // int: 0 no newest version, 1 current, 2 update available, 3 downgrade
};

// What a collapsed separator shows of the mods inside it ([Settings]
// collapsible_separators_*; MO2's ModListView::modFlags / conflictFlags / contents /
// markerColor / refreshMarkersAndPlugins): their icons in the Flags (icons_2), Conflicts
// (icons_1) and Content (icons_3) columns, their highlight colour averaged (conflicts_to),
// and a selected one highlights what everything inside conflicts with (conflicts_from).
struct RollUps
{
  bool flags         = true;
  bool conflicts     = true;
  bool contents      = true;
  bool highlightTo   = true;
  bool highlightFrom = true;
};

// A separator row (column 0) that is collapsed and has rows inside.
bool isCollapsedSeparator(const QTreeView* view, const QModelIndex& index);
// Calls `f` with column 0 of every row inside `separator`, nested separators included.
void forEachDescendant(const QModelIndex& separator, const std::function<void(const QModelIndex&)>& f);

// The conflict index's view of one row; applied after the index is built or updated.
void applyConflicts(QStandardItem* nameItem, const strata::ConflictRow& row);

enum Kind
{
  Regular,
  Separator,
  Foreign,
  Backup,
  Overwrite
};

// ModList::getColumnName.
QStringList columnNames();

// Replaces the model's content with `rows`: nested under their separators when `nested`
// (ascending priority, Leet change 9), else one flat list.
// `byName`, when given, receives each row's name item by internal name.
void fill(QStandardItemModel& model, const rust::Vec<strata::ModRow>& rows, bool nested,
          const QString& gameName, QHash<QString, QStandardItem*>* byName = nullptr);

// Updates one row in place (a meta.ini change: notes, colour...), no reset.
void update(QStandardItem* nameItem, const strata::ModRow& row, const QString& gameName);

// Enabled regular mods among the rows `view`'s model shows (the "Active:" counter,
// ModListView::counters' visible.active).
int visibleActiveCount(const QAbstractItemModel& model);

}  // namespace modlist

// ModListSortProxy, the parts that exist so far: the text filter and foreign-mod hiding,
// sorting by priority (fixed places for backups and Overwrite) or by text.
class ModListProxy : public QSortFilterProxyModel
{
  Q_OBJECT

public:
  explicit ModListProxy(QObject* parent = nullptr);

  void setFilterText(const QString& text);
  void setDisplayForeign(bool display);
  // A hidden column's text is not searched (m_EnabledColumns).
  void setColumnVisible(int column, bool visible);
  // The filter panel's verdict: only these mods (by internal name) pass; none = no criteria.
  void setCriteriaPass(std::optional<QSet<QString>> names);

  // Drag and drop as MO2's ModList (modlistdropinfo.cpp): the rows of a drag from the list
  // carry the text "mod" (ModListDropInfo::ModText), rows of the download list "download";
  // local files and folders (archives to install, folders to copy) come as URLs.
  QStringList mimeTypes() const override;
  QMimeData* mimeData(const QModelIndexList& indexes) const override;
  bool canDropMimeData(const QMimeData* data, Qt::DropAction action, int row, int column,
                       const QModelIndex& parent) const override;

protected:
  bool filterAcceptsRow(int row, const QModelIndex& parent) const override;
  bool lessThan(const QModelIndex& left, const QModelIndex& right) const override;

private:
  template <class Change>
  void refilter(Change change);

  QString m_filter;
  std::optional<QSet<QString>> m_criteriaPass;
  bool m_displayForeign = true;
  bool m_visible[modlist::ColumnCount];
};

// IconDelegate (MO2's uibase): the icons of IconsRole side by side, 16 px at most; an empty
// name keeps its slot unless the column is narrower than `compactSize`.
class IconDelegate : public QStyledItemDelegate
{
public:
  // `rollUps` (owned by the window, may be null): a collapsed separator shows the icons of
  // the rows inside it
  IconDelegate(QTreeView* view, int column, int compactSize, const modlist::RollUps* rollUps = nullptr);
  void paint(QPainter* painter, const QStyleOptionViewItem& option,
             const QModelIndex& index) const override;
  QSize sizeHint(const QStyleOptionViewItem& option, const QModelIndex& index) const override;

private:
  QStringList icons(const QModelIndex& index) const;

  QTreeView* m_view;
  int m_column;
  int m_compactSize;
  const modlist::RollUps* m_rollUps;
};

// ModListStyledItemDelegate: separator colours with a readable text colour, Sub-Separator
// '#' per level and "(n)" counts, mods inside a separator moved one indentation left.
class ModListDelegate : public QStyledItemDelegate
{
public:
  // `markers`: the selection's conflict highlight by internal name (ModListView::markerColor),
  // owned by the window.
  ModListDelegate(QTreeView* view, bool alwaysShowSize, const QHash<QString, QColor>* markers,
                  const modlist::RollUps* rollUps = nullptr);

  void initStyleOption(QStyleOptionViewItem* option, const QModelIndex& index) const override;
  void paint(QPainter* painter, const QStyleOptionViewItem& option,
             const QModelIndex& index) const override;

private:
  QTreeView* m_view;
  bool m_alwaysShowSize;
  const QHash<QString, QColor>* m_markers;
  const modlist::RollUps* m_rollUps;
};

#pragma once

// The categories / filter panel left of the mod list (MO2's FilterList): the special
// criteria, one "<Contains ...>" per content, the categories mods have; a click cycles an
// entry through off, on, inverted (right click backwards); And / Or; what separators do.

#include "strata_bridge/bridge.h"

#include <QObject>

class QComboBox;
class QPushButton;
class QRadioButton;
class QTreeWidget;
class QTreeWidgetItem;

class FilterPanel : public QObject
{
  Q_OBJECT

public:
  FilterPanel(QTreeWidget* tree, QRadioButton* andButton, QRadioButton* orButton,
              QComboBox* separators, QPushButton* clear, QObject* parent = nullptr);

  // Fills the entries again (FilterList::refresh), keeping what was ticked.
  void setEntries(const rust::Vec<strata::FilterEntry>& entries);

  rust::Vec<strata::Criterion> criteria() const;
  bool isAnd() const;
  // 0 filter, 1 show, 2 hide (ModListSortProxy::SeparatorsMode)
  std::uint8_t separators() const;
  bool active() const;
  // The ticked entries' names, for the label under the list.
  QStringList activeNames() const;

  // Test hook: ticks the entry named `name` (as one click does).
  void tick(const QString& name);

signals:
  void changed();

protected:
  bool eventFilter(QObject* watched, QEvent* event) override;

private:
  void cycle(QTreeWidgetItem* item, int step);

  QTreeWidget* m_tree;
  QRadioButton* m_and;
  QComboBox* m_separators;
};

// Placeholder for MO2's MOBase::SortableTreeWidget, which mainwindow.ui names.
// The class name must stay: MO2's stylesheets select on it. Behaviour comes in M3.
#pragma once

#include <QTreeWidget>

namespace MOBase
{

class SortableTreeWidget : public QTreeWidget
{
  Q_OBJECT

public:
  using QTreeWidget::QTreeWidget;
};

}  // namespace MOBase

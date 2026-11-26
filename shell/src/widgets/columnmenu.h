#pragma once

// MOBase::setCustomizableColumns (uibase widgetutility.cpp) and DownloadListView's copy: a
// right click on a list's header opens a menu with a checkbox per column (all but the
// first) that shows or hides it; the columns can be dragged into another order. The header's
// state, hidden columns included, is saved by LayoutState as MO2's.

#include <QCheckBox>
#include <QHeaderView>
#include <QMenu>
#include <QTreeView>
#include <QWidgetAction>

namespace strata
{

inline void setCustomizableColumns(QTreeView* view)
{
  auto* header = view->header();
  header->setSectionsMovable(true);
  header->setContextMenuPolicy(Qt::CustomContextMenu);
  QObject::connect(header, &QWidget::customContextMenuRequested, view, [view](const QPoint& pos) {
    auto* header = view->header();
    QMenu menu;
    menu.setObjectName("columnMenu");
    QAbstractItemModel* model = header->model();
    for (int i = 1; model && i < model->columnCount(); ++i) {
      auto* box = new QCheckBox(&menu);
      box->setText(model->headerData(i, Qt::Horizontal).toString());
      box->setChecked(!header->isSectionHidden(i));
      QObject::connect(box, &QCheckBox::clicked, header, [header, box, i] { header->setSectionHidden(i, !box->isChecked()); });
      auto* action = new QWidgetAction(&menu);
      action->setDefaultWidget(box);
      menu.addAction(action);
    }
    menu.exec(header->viewport()->mapToGlobal(pos));
  });
}

}  // namespace strata

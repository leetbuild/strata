// MO2's PluginListView, which mainwindow.ui names. The class name must stay: MO2's
// stylesheets select on it. The model and its behaviour are set by PluginsTab.
#pragma once

#include <QDropEvent>
#include <QMimeData>
#include <QTreeView>

class PluginListView : public QTreeView
{
  Q_OBJECT

public:
  explicit PluginListView(QWidget* parent = nullptr) : QTreeView(parent)
  {
    setDragEnabled(true);
    setAcceptDrops(true);
    setDropIndicatorShown(true);
    setDragDropMode(QAbstractItemView::DragDrop);
    setDefaultDropAction(Qt::MoveAction);
  }

signals:
  // The selected plugins were dropped above the row `row` of the view's model (-1 = the
  // end). The model is never changed by the drop itself: the core decides.
  void dropRequested(int row);

protected:
  void dropEvent(QDropEvent* event) override
  {
    // rows of this list (PluginList::dropMimeData reads the model's own row data)
    if (event->mimeData()->hasUrls() ||
        !event->mimeData()->hasFormat("application/x-qstandarditemmodeldatalist")) {
      event->ignore();
      return;
    }
    const QModelIndex index = indexAt(event->position().toPoint());
    int row                 = -1;
    if (index.isValid()) {
      row = dropIndicatorPosition() == BelowItem ? index.row() + 1 : index.row();
      if (row >= model()->rowCount()) {
        row = -1;
      }
    }
    event->setDropAction(Qt::IgnoreAction);
    event->accept();
    stopAutoScroll();
    setState(NoState);
    viewport()->update();
    emit dropRequested(row);
  }
};

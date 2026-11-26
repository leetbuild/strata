// MO2's ModListView, which mainwindow.ui names. The class name must stay: MO2's stylesheets
// select on it. The model, proxy and delegate are set by the main window (modlist.h).
#pragma once

#include <QDropEvent>
#include <QItemSelection>
#include <utility>
#include <QKeyEvent>

#include <functional>
#include <QMimeData>
#include <QMouseEvent>
#include <QUrl>
#include <QTreeView>

class ModListView : public QTreeView
{
  Q_OBJECT

public:
  explicit ModListView(QWidget* parent = nullptr) : QTreeView(parent)
  {
    setDragEnabled(true);
    setAcceptDrops(true);
    setDropIndicatorShown(true);
    setDragDropMode(QAbstractItemView::DragDrop);
    setDefaultDropAction(Qt::MoveAction);
  }

signals:
  // The selected mods were dropped (in the view's model's coordinates): under `parent` at
  // `row` (row -1 = onto the separator `parent` itself), and whether the drop indicator was
  // just below an expanded separator. The model is never changed by the drop itself: the
  // main window asks the core, then rebuilds from its answer.
  void dropRequested(const QModelIndex& parent, int row, bool belowExpandedSeparator);
  // Something from outside the list was dropped at the same kind of place: local files or
  // folders (`urls`) or a row of the download list (`download`, its file name). Handled
  // after the drop has finished (MO2 queues it, so an installer dialog can take drops).
  void foreignDropRequested(const QList<QUrl>& urls, const QString& download,
                            const QModelIndex& parent, int row, bool belowExpandedSeparator);

  // Keys of ModListView::event, for the window to carry out: Ctrl+Shift+Enter on a
  // separator (its level), Ctrl+Enter (open the folder), Ctrl+Up / Ctrl+Down (move the
  // selection, sorted by priority; `up` true for Up), Delete (remove), Space (toggle).
  void levelRequested(const QModelIndex& separator);
  void openFolderRequested(const QModelIndex& index);
  void moveRequested(bool up);
  void removeRequested();
  void toggleRequested();

public:
  // [Plugins] Sub-Separator\invertAltModifierWhenSeparatorIsCollapsed: on a closed
  // separator a plain click selects its content and Alt+click does not.
  void setInvertAltOnCollapsed(bool invert) { m_invertAlt = invert; }

  // ModListView::visualRect: matches what the delegate draws (mods inside a separator one
  // indentation to the left; room for the branch arrow when the indentation is 0), so
  // clicks land where the user sees the item.
  QRect visualRect(const QModelIndex& index) const override
  {
    QRect rect = QTreeView::visualRect(index);
    if (index.isValid() && index.column() == 0) {
      if (indentation() == 0) {
        rect.adjust(rect.height(), 0, 0, 0);
      } else if (index.parent().isValid() &&
                 index.data(Qt::UserRole + 1).toInt() != 1 /* modlist::Separator */) {
        rect.adjust(-indentation(), 0, 0, 0);
      }
    }
    return rect;
  }

protected:
  void dropEvent(QDropEvent* event) override
  {
    // ModListDropInfo: what was dropped is told by the data, not by the source widget
    const QMimeData* mime = event->mimeData();
    const bool fromList   = !mime->hasUrls() && mime->text() == "mod";
    const QString download = mime->hasUrls() || mime->text() != "download"
                                 ? QString()
                                 : QString::fromUtf8(mime->data("application/x-strata-download"));
    if (!fromList && !mime->hasUrls() && download.isEmpty()) {
      event->ignore();
      return;
    }
    const QModelIndex index = indexAt(event->position().toPoint()).siblingAtColumn(0);
    QModelIndex parent;
    int row    = -1;
    bool below = false;
    switch (dropIndicatorPosition()) {
    case OnItem:
      if (index.data(Qt::UserRole + 1).toInt() == 1 /* modlist::Separator */) {
        parent = index;
      } else {
        parent = index.parent();
        row    = index.row();
      }
      break;
    case AboveItem:
      parent = index.parent();
      row    = index.row();
      break;
    case BelowItem:
      parent = index.parent();
      row    = index.row() + 1;
      below  = isExpanded(index) && model()->hasChildren(index);
      break;
    case OnViewport:
      row = model()->rowCount();
      break;
    }
    event->setDropAction(fromList ? Qt::IgnoreAction : Qt::CopyAction);
    event->accept();
    stopAutoScroll();
    setState(NoState);
    viewport()->update();
    if (fromList) {
      emit dropRequested(parent, row, below);
    } else {
      emit foreignDropRequested(mime->urls(), download, parent, row, below);
    }
  }

  // ModListView::onBranchClicked: with indentation 0 the delegate draws the arrow at the
  // left of the name; a click there opens or closes the separator.
  bool onBranchClicked(QMouseEvent* event)
  {
    if (indentation() == 0) {
      const QModelIndex index = indexAt(event->position().toPoint());
      if (index.isValid() && index.column() == 0 && model()->hasChildren(index)) {
        const QRect row = QTreeView::visualRect(index);
        if (event->position().x() < row.x() + row.height()) {
          if (event->type() == QEvent::MouseButtonPress) {
            m_branchPress = true;
            setExpanded(index, !isExpanded(index));
          }
          return true;
        }
      }
    }
    return false;
  }

  // ModListView::selectSeparatorContents (Leet change 9): Alt+click on a separator selects
  // (or deselects, following the separator) everything inside it, nested ones included.
  void selectSeparatorContents(QMouseEvent* event)
  {
    const QModelIndex index = indexAt(event->position().toPoint()).siblingAtColumn(0);
    if (!index.isValid() || !model()->hasChildren(index)) {
      return;
    }
    bool alt = event->modifiers() & Qt::AltModifier;
    if (!isExpanded(index) && m_invertAlt) {
      alt = !alt;
    }
    if (!alt) {
      return;
    }
    const auto flag = selectionModel()->isSelected(index) ? QItemSelectionModel::Select
                                                          : QItemSelectionModel::Deselect;
    QItemSelection selection;
    std::function<void(const QModelIndex&)> add = [&](const QModelIndex& parent) {
      const int rows = model()->rowCount(parent);
      selection.merge(QItemSelection(model()->index(0, 0, parent), model()->index(rows - 1, 0, parent)),
                      QItemSelectionModel::Select);
      for (int r = 0; r < rows; ++r) {
        if (model()->hasChildren(model()->index(r, 0, parent))) {
          add(model()->index(r, 0, parent));
        }
      }
    };
    add(index);
    selectionModel()->select(selection, flag | QItemSelectionModel::Rows);
  }

  // ModListView::mousePressEvent / mouseReleaseEvent: the branch arrow, no editing with Alt,
  // then the separator's content (in both: the selection is not always made on press)
  void mousePressEvent(QMouseEvent* event) override
  {
    m_branchPress = false;
    if (onBranchClicked(event)) {
      return;
    }
    const auto triggers = editTriggers();
    if (event->modifiers() & Qt::AltModifier) {
      setEditTriggers(NoEditTriggers);
    }
    QTreeView::mousePressEvent(event);
    setEditTriggers(triggers);
    selectSeparatorContents(event);
  }

  // A press on the arrow never reaches QAbstractItemView, which still holds the position of
  // an older press (the top of the list after start): a move while the button is down would
  // drag-select from there to the arrow (MO2 has the same bug). Moves of that press are
  // dropped until the release.
  void mouseMoveEvent(QMouseEvent* event) override
  {
    if (m_branchPress) {
      if (!(event->buttons() & Qt::LeftButton)) {
        m_branchPress = false;
      }
      return;
    }
    QTreeView::mouseMoveEvent(event);
  }

  void mouseReleaseEvent(QMouseEvent* event) override
  {
    const bool branchPress = std::exchange(m_branchPress, false);
    if (onBranchClicked(event) || branchPress) {
      return;
    }
    const auto triggers = editTriggers();
    if (event->modifiers() & Qt::AltModifier) {
      setEditTriggers(NoEditTriggers);
    }
    QTreeView::mouseReleaseEvent(event);
    setEditTriggers(triggers);
    selectSeparatorContents(event);
  }

  // ModListView::event: the list's keys
  bool event(QEvent* event) override
  {
    if (event->type() != QEvent::KeyPress || !selectionModel() || !selectionModel()->hasSelection()) {
      return QTreeView::event(event);
    }
    auto* key        = static_cast<QKeyEvent*>(event);
    const bool enter = key->key() == Qt::Key_Return || key->key() == Qt::Key_Enter;
    const QModelIndex index = selectionModel()->currentIndex().siblingAtColumn(0);
    const int single        = selectionModel()->selectedRows().count() == 1;
    if (key->modifiers() == (Qt::ControlModifier | Qt::ShiftModifier) && enter && index.isValid()) {
      if (index.data(Qt::UserRole + 1).toInt() == 1 /* modlist::Separator */) {
        emit levelRequested(index);
        return true;
      }
    } else if (key->modifiers() == Qt::ControlModifier) {
      if (enter && single) {
        emit openFolderRequested(index);
        return true;
      }
      if (key->key() == Qt::Key_Up || key->key() == Qt::Key_Down) {
        emit moveRequested(key->key() == Qt::Key_Up);
        return true;
      }
    } else if (key->modifiers() == Qt::ShiftModifier) {
      if (enter && single) {
        if (model()->hasChildren(index)) {
          setExpanded(index, !isExpanded(index));
        } else if (index.parent().isValid()) {
          setExpanded(index.parent(), false);
          selectionModel()->select(index.parent(), QItemSelectionModel::SelectCurrent | QItemSelectionModel::Rows);
          setCurrentIndex(index.parent());
        }
        return true;
      }
    } else if (key->modifiers() == Qt::NoModifier || key->modifiers() == Qt::KeypadModifier) {
      if (key->key() == Qt::Key_Delete) {
        emit removeRequested();
        return true;
      }
      if (key->key() == Qt::Key_Space) {
        emit toggleRequested();
        return true;
      }
    }
    return QTreeView::event(event);
  }

private:
  bool m_invertAlt = false;
  bool m_branchPress = false;  // the left button went down on a separator's arrow
};

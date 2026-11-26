// Helpers that drive the real widgets the way a user does: mouse clicks on check boxes and
// branch arrows, typed text, keys, drops (the same QDragEnter/Move/Drop events Qt sends
// for a real drag), context menus answered through the menu's own keyboard handling, and
// modal dialogs answered while they block. Used by the tests in drive.cpp.
#pragma once

#include <QAbstractButton>
#include <QAbstractItemView>
#include <QApplication>
#include <QComboBox>
#include <QLineEdit>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDir>
#include <QFile>
#include <QMenu>
#include <QMimeData>
#include <QMouseEvent>
#include <QPointer>
#include <QPushButton>
#include <QStyleOptionButton>
#include <QStyleOptionViewItem>
#include <QStyledItemDelegate>
#include <QTest>
#include <QTimer>
#include <QTreeView>

#include <functional>

namespace drive
{

inline QString fileText(const QString& path)
{
  QFile f(path);
  return f.open(QIODevice::ReadOnly) ? QString::fromUtf8(f.readAll()) : QString();
}

// The non-comment lines of a MO2 list file, CR removed.
inline QStringList listLines(const QString& path)
{
  QStringList out;
  for (QString line : fileText(path).split('\n')) {
    line.remove('\r');
    if (!line.isEmpty() && !line.startsWith('#')) {
      out << line;
    }
  }
  return out;
}

// The first index under `parent` (depth first) whose `role` equals `value`, in column 0.
inline QModelIndex find(const QAbstractItemModel* model, int role, const QVariant& value,
                        const QModelIndex& parent = {})
{
  for (int r = 0; r < model->rowCount(parent); ++r) {
    const QModelIndex index = model->index(r, 0, parent);
    if (index.data(role) == value) {
      return index;
    }
    if (model->hasChildren(index)) {
      if (QModelIndex found = find(model, role, value, index); found.isValid()) {
        return found;
      }
    }
  }
  return {};
}

// initStyleOption is protected: reached through a member pointer taken in a subclass.
struct OpenDelegate : QStyledItemDelegate
{
  static auto init() { return &OpenDelegate::initStyleOption; }
};

// Where the check box of `index` is drawn: the delegate's own option, as
// QStyledItemDelegate::editorEvent computes it when it decides whether a click toggles.
inline QPoint checkBoxCenter(QAbstractItemView* view, const QModelIndex& index)
{
  QStyleOptionViewItem option;
  option.initFrom(view->viewport());
  option.rect = view->visualRect(index);
  option.widget = view;
  if (auto* d = qobject_cast<QStyledItemDelegate*>(view->itemDelegateForIndex(index))) {
    (d->*OpenDelegate::init())(&option, index);
  }
  return view->style()->subElementRect(QStyle::SE_ItemViewItemCheckIndicator, &option, view).center();
}

inline void clickCheckBox(QAbstractItemView* view, const QModelIndex& index)
{
  view->scrollTo(index);
  QTest::mouseClick(view->viewport(), Qt::LeftButton, Qt::NoModifier, checkBoxCenter(view, index));
}

// A click on a check box's indicator (QCheckBox takes clicks on its indicator and label
// only, not on the empty rest of a wide box).
inline void clickBox(QAbstractButton* box)
{
  QStyleOptionButton option;
  option.initFrom(box);
  const QRect r = box->style()->subElementRect(QStyle::SE_CheckBoxIndicator, &option, box);
  QTest::mouseClick(box, Qt::LeftButton, Qt::NoModifier, r.isValid() ? r.center() : QPoint(8, box->height() / 2));
}

// A click on the row's text (selects it, as a user does before a drag or a right click).
inline void clickRow(QAbstractItemView* view, const QModelIndex& index,
                     Qt::KeyboardModifiers modifiers = Qt::NoModifier)
{
  view->scrollTo(index);
  const QRect r = view->visualRect(index);
  QTest::mouseClick(view->viewport(), Qt::LeftButton, modifiers, QPoint(r.right() - 4, r.center().y()));
}

// A double click as the platform delivers it: press, release, double-click (in place of the
// second press), release. (QTest::mouseDClick leaves a tree view in its drag-select state,
// where QTreeView ignores the double click.)
inline void doubleClick(QWidget* widget, QPoint pos)
{
  QTest::mouseClick(widget, Qt::LeftButton, Qt::NoModifier, pos);
  const QPointF local(pos), global(widget->mapToGlobal(pos));
  QMouseEvent dbl(QEvent::MouseButtonDblClick, local, global, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
  QApplication::sendEvent(widget, &dbl);
  QMouseEvent release(QEvent::MouseButtonRelease, local, global, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
  QApplication::sendEvent(widget, &release);
}

// The events of a drag that ends over `pos` in the view's viewport: enter, move (the view
// computes its drop indicator there), drop. Returns whether the drop was accepted.
inline bool dropAt(QAbstractItemView* view, QMimeData* mime, QPoint pos,
                   Qt::DropAction action = Qt::MoveAction)
{
  QWidget* target = view->viewport();
  const Qt::DropActions actions = Qt::CopyAction | Qt::MoveAction;
  QDragEnterEvent enter(QPointF(pos), actions, mime, Qt::LeftButton, Qt::NoModifier);
  enter.setDropAction(action);
  QApplication::sendEvent(target, &enter);
  if (!enter.isAccepted()) {
    qWarning("drive: the drag was not accepted on entering");
    return false;
  }
  QDragMoveEvent move(QPointF(pos), actions, mime, Qt::LeftButton, Qt::NoModifier);
  move.setDropAction(action);
  QApplication::sendEvent(target, &move);
  if (!move.isAccepted()) {
    qWarning("drive: the drag was not accepted at (%d, %d)", pos.x(), pos.y());
    return false;
  }
  QDropEvent drop(QPointF(pos), actions, mime, Qt::LeftButton, Qt::NoModifier);
  drop.setDropAction(action);
  QApplication::sendEvent(target, &drop);
  return drop.isAccepted();
}

// The selected rows of `view` dragged onto `pos` (the mime data the view's model makes for
// them, as QAbstractItemView::startDrag does).
inline bool dragSelectionTo(QAbstractItemView* view, QPoint pos)
{
  QModelIndexList indexes = view->selectionModel()->selectedIndexes();
  std::unique_ptr<QMimeData> mime(view->model()->mimeData(indexes));
  return mime && dropAt(view, mime.get(), pos);
}

// Middle of a row (on the item), its upper or lower edge (above / below it).
inline QPoint onRow(QAbstractItemView* view, const QModelIndex& index)
{
  view->scrollTo(index);
  return view->visualRect(index).center();
}
inline QPoint aboveRow(QAbstractItemView* view, const QModelIndex& index)
{
  view->scrollTo(index);
  const QRect r = view->visualRect(index);
  return QPoint(r.center().x(), r.top() + 1);
}

// ---- modal dialogs and menus: answered from a timer while they block

// Calls `act` once `ready` holds (polled every 20 ms from the event loop). A handler must
// return to the loop it was called from before the program can go on (a dialog's exec()
// returns only then), so a sequence of clicks in one dialog is a chain of these.
inline void when(std::function<bool()> ready, std::function<void()> act, int timeoutMs = 10000)
{
  auto* timer   = new QTimer(qApp);
  auto deadline = std::make_shared<QDeadlineTimer>(timeoutMs);
  QObject::connect(timer, &QTimer::timeout, qApp, [=] {
    if (ready()) {
      timer->stop();
      timer->deleteLater();
      act();
    } else if (deadline->hasExpired()) {
      qWarning("drive: gave up waiting after %d ms", timeoutMs);
      timer->stop();
      timer->deleteLater();
    }
  });
  timer->start(20);
}

// Calls `answer` with the next modal widget that `accept` takes (default: any) which was
// not handed to an earlier handler. Polls every 20 ms; gives up after `timeoutMs`.
inline void whenModal(std::function<void(QWidget*)> answer,
                      std::function<bool(QWidget*)> accept = {}, int timeoutMs = 10000)
{
  static QList<QPointer<QWidget>> seen;
  auto* timer   = new QTimer(qApp);
  auto deadline = std::make_shared<QDeadlineTimer>(timeoutMs);
  QObject::connect(timer, &QTimer::timeout, qApp, [=] {
    QWidget* w = QApplication::activeModalWidget();
    if (accept && (!w || !accept(w))) {
      // a window that runs its own event loop (the FOMOD dialog) need not be modal
      w = nullptr;
      for (QWidget* top : QApplication::topLevelWidgets()) {
        if (qobject_cast<QDialog*>(top) && top->isVisible() && !seen.contains(top) && accept(top)) {
          w = top;
        }
      }
    }
    if (w && !seen.contains(w) && w->isVisible() && (!accept || accept(w))) {
      seen << w;
      timer->stop();
      timer->deleteLater();
      QPointer<QWidget> guard(w);
      qDebug("drive: answering %s \"%s\"", w->metaObject()->className(), qUtf8Printable(w->objectName()));
      answer(w);
      if (guard && guard->isVisible() && QTest::currentTestFailed()) {
        // a check in the answer failed: close the dialog, or the test would hang in it
        if (auto* dialog = qobject_cast<QDialog*>(guard.data())) {
          dialog->reject();
        } else {
          guard->close();
        }
      }
      return;
    }
    if (deadline->hasExpired()) {
      qWarning("drive: no dialog came within %d ms", timeoutMs);
      timer->stop();
      timer->deleteLater();
    }
  });
  timer->start(20);
}

template <class T>
void whenModalOf(std::function<void(T*)> answer, int timeoutMs = 10000)
{
  whenModal([answer](QWidget* w) { answer(qobject_cast<T*>(w)); },
            [](QWidget* w) { return qobject_cast<T*>(w) != nullptr; }, timeoutMs);
}

// A picture of a dialog as the test saw it, in _stage/shell-drive/shots/<name>.png.
inline void shoot(QWidget* w, const QString& name)
{
  const QString dir = qEnvironmentVariable("STRATA_REPO") + "/_stage/shell-drive/shots";
  QDir().mkpath(dir);
  w->grab().save(dir + "/" + name + ".png");
}

// Types `name` into an installer's editable name combo as a user does. The combo completes
// inline from its entries ("Picked" + " Mod" selected when "Picked Mod" is one of them);
// like a user, Delete removes such a completion. Checks the text is exactly `name`.
inline void typeName(QComboBox* combo, const QString& name)
{
  QLineEdit* edit = combo->lineEdit();
  edit->setFocus();
  edit->selectAll();
  QTest::keyClicks(edit, name);
  if (edit->text() != name && edit->hasSelectedText()) {
    QTest::keyClick(edit, Qt::Key_Delete);
  }
  QCOMPARE(edit->text(), name);
}

// A dialog made from a Designer form, by the form's object name ("SimpleInstallDialog").
inline std::function<bool(QWidget*)> named(const QString& objectName)
{
  return [objectName](QWidget* w) { return w->objectName() == objectName; };
}

inline std::function<bool(QWidget*)> titled(const QString& title)
{
  return [title](QWidget* w) { return w->windowTitle() == title; };
}

// Clicks the button of a dialog box with that role (QMessageBox, QInputDialog...).
inline void clickButton(QWidget* dialog, QDialogButtonBox::StandardButton which)
{
  if (auto* box = dialog->findChild<QDialogButtonBox*>()) {
    if (QPushButton* b = box->button(which)) {
      QTest::mouseClick(b, Qt::LeftButton);
      return;
    }
  }
  qWarning("drive: no such button in %s", qUtf8Printable(dialog->windowTitle()));
}

// The text of a menu entry without its accelerator ampersands.
inline QString plain(QString text)
{
  return text.remove('&').trimmed();
}

// Picks `path` ("Send to.../Top") in the next popup menu with the menu's own keys: the
// entry is made active and Return (or Right, for a sub-menu) is pressed, so QMenu hides
// and triggers it as for a user. Records in `*found` whether every entry existed and was
// enabled (a missing one closes the menu).
inline void whenMenu(const QStringList& path, bool* found = nullptr, int timeoutMs = 10000)
{
  auto step     = std::make_shared<int>(0);
  auto timer    = new QTimer(qApp);
  auto deadline = std::make_shared<QDeadlineTimer>(timeoutMs);
  auto last     = std::make_shared<QPointer<QMenu>>();
  if (found) {
    *found = false;
  }
  QObject::connect(timer, &QTimer::timeout, qApp, [=] {
    auto* menu = qobject_cast<QMenu*>(QApplication::activePopupWidget());
    if (!menu || menu == *last || !menu->isVisible()) {
      if (deadline->hasExpired()) {
        qWarning("drive: no menu came for \"%s\"", qUtf8Printable(path.join('/')));
        timer->deleteLater();
      }
      return;
    }
    QAction* action = nullptr;
    for (QAction* a : menu->actions()) {
      if (plain(a->text()) == path[*step]) {
        action = a;
      }
    }
    if (!action || !action->isEnabled()) {
      qWarning("drive: no enabled entry \"%s\" in the menu", qUtf8Printable(path[*step]));
      timer->deleteLater();
      menu->close();
      return;
    }
    *last = menu;
    menu->setActiveAction(action);
    if (++*step == path.size()) {
      if (found) {
        *found = true;
      }
      timer->deleteLater();
      QTest::keyClick(menu, Qt::Key_Return);
    } else {
      QTest::keyClick(menu, Qt::Key_Right);  // opens the sub-menu
    }
  });
  timer->start(20);
}

// The context menu of a list, as a right click on that row asks for it.
inline void contextMenu(QAbstractItemView* view, const QModelIndex& index)
{
  const QPoint pos = index.isValid() ? onRow(view, index) : QPoint(5, view->viewport()->height() - 5);
  QContextMenuEvent event(QContextMenuEvent::Mouse, pos, view->viewport()->mapToGlobal(pos));
  QApplication::sendEvent(view->viewport(), &event);
}

}  // namespace drive

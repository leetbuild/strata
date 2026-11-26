#pragma once

// First-start column widths for a list with no saved header (the mod list, the plugin list).
// MO2 2.5.2 (ModListView::setup, MainWindow::resizeLists / allowListResize) fits every
// visible column to its contents and stretches the name column; at its default window size
// that pushes columns out of view when the contents are long. Here the name column keeps at
// least 40% of the width (45% of it, up to 24 letters, when there is room) and every visible column stays inside the view: icon columns get
// what their icons need, text columns fit their contents and share what is left when that
// is too much. The fit follows the view's width until the user touches the header.

#include <QObject>

class LayoutState;
class QTreeView;

class ColumnFit : public QObject
{
  Q_OBJECT

public:
  ColumnFit(QTreeView* view, int nameColumn, LayoutState* layout);

  // Fits now, and again whenever the view's width changes, until the user presses a mouse
  // button on the header (a drag or a double click on a section edge).
  void start();

  // One fit of `view` to its viewport's width.
  static void fit(QTreeView* view, int nameColumn);

protected:
  bool eventFilter(QObject* watched, QEvent* event) override;

private:
  void fitQuietly();

  QTreeView* m_view;
  int m_name;
  LayoutState* m_layout;
  bool m_active = false;
};

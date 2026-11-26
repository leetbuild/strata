// Placeholder for MO2's DownloadListView, which mainwindow.ui names.
// The class name must stay: MO2's stylesheets select on it. Behaviour comes in M3.
#pragma once

#include <QTreeView>

class DownloadListView : public QTreeView
{
  Q_OBJECT

public:
  using QTreeView::QTreeView;
};

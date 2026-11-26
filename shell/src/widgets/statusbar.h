// Placeholder for MO2's StatusBar, which mainwindow.ui names.
// The class name must stay: MO2's stylesheets select on it. Behaviour comes in M3.
#pragma once

#include <QStatusBar>

class StatusBar : public QStatusBar
{
  Q_OBJECT

public:
  using QStatusBar::QStatusBar;
};

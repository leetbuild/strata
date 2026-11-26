// Placeholder for MO2's LinkLabel, which mainwindow.ui names.
// The class name must stay: MO2's stylesheets select on it. Behaviour comes in M3.
#pragma once

#include <QColor>
#include <QLabel>

class LinkLabel : public QLabel
{
  Q_OBJECT
  // Set by mainwindow.ui and by stylesheets (qproperty-linkColor).
  Q_PROPERTY(QColor linkColor MEMBER m_linkColor)

public:
  using QLabel::QLabel;

private:
  QColor m_linkColor;
};

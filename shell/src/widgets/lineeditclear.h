// Placeholder for MO2's MOBase::LineEditClear, which mainwindow.ui names.
// The class name must stay: MO2's stylesheets select on it. Behaviour comes in M3.
#pragma once

#include <QLineEdit>

namespace MOBase
{

class LineEditClear : public QLineEdit
{
  Q_OBJECT

public:
  using QLineEdit::QLineEdit;
};

}  // namespace MOBase

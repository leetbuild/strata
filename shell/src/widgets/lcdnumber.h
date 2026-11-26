// Placeholder for MO2's LCDNumber, which mainwindow.ui names.
// The class name must stay: MO2's stylesheets select on it. Behaviour comes in M3.
#pragma once

#include <QLCDNumber>

class LCDNumber : public QLCDNumber
{
  Q_OBJECT

public:
  using QLCDNumber::QLCDNumber;
};

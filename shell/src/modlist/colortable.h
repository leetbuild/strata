#pragma once

// MO2's ColorTable (colortable.h) as the settings form names it: the table of the mod
// list's highlight colours. The settings dialog fills it with the stored colours
// (`highlight_colors`); editing them is not in Strata yet, so it stays disabled.

#include <QTableWidget>

class ColorTable : public QTableWidget
{
  Q_OBJECT

public:
  explicit ColorTable(QWidget* parent = nullptr) : QTableWidget(parent) {}
};

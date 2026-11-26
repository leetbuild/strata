// The two custom widgets of MO2's images tab (modinfodialogimages.h), as modinfodialog.ui
// names them. MO2 paints its thumbnails itself; Strata puts a QListWidget in icon mode inside
// ThumbnailsWidget instead (ModInfoDialog::setupImages) and leaves Scrollbar hidden.
#pragma once

#include <QScrollBar>
#include <QWidget>

namespace ImagesTabHelpers
{

class ThumbnailsWidget : public QWidget
{
  Q_OBJECT

public:
  using QWidget::QWidget;
};

class Scrollbar : public QScrollBar
{
  Q_OBJECT

public:
  using QScrollBar::QScrollBar;
};

}  // namespace ImagesTabHelpers

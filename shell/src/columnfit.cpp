#include "columnfit.h"

#include "layoutstate.h"
#include "modlist.h"

#include <QEvent>
#include <QHeaderView>
#include <QTimer>
#include <QTreeView>

#include <algorithm>
#include <vector>

ColumnFit::ColumnFit(QTreeView* view, int nameColumn, LayoutState* layout)
    : QObject(view), m_view(view), m_name(nameColumn), m_layout(layout)
{
  m_view->viewport()->installEventFilter(this);
  m_view->header()->viewport()->installEventFilter(this);
}

void ColumnFit::start()
{
  m_active = true;
  // after the window's first layout: the viewport has its real width then
  QTimer::singleShot(0, this, [this] { fitQuietly(); });
}

void ColumnFit::fitQuietly()
{
  if (!m_active) {
    return;
  }
  LayoutState::Quiet quiet(*m_layout);  // the program's widths, not the user's: nothing saved
  fit(m_view, m_name);
}

bool ColumnFit::eventFilter(QObject* watched, QEvent* event)
{
  if (watched == m_view->header()->viewport() &&
      (event->type() == QEvent::MouseButtonPress || event->type() == QEvent::MouseButtonDblClick)) {
    m_active = false;  // the user sizes the columns from now on
  } else if (watched == m_view->viewport() && event->type() == QEvent::Resize && m_active) {
    QTimer::singleShot(0, this, [this] { fitQuietly(); });
  }
  return QObject::eventFilter(watched, event);
}

void ColumnFit::fit(QTreeView* view, int nameColumn)
{
  QHeaderView* header = view->header();
  const int available = view->viewport()->width();
  if (available <= 0) {
    return;
  }
  const QFontMetrics metrics = view->fontMetrics();
  const int least            = header->minimumSectionSize();
  const int readable         = metrics.horizontalAdvance(QLatin1Char('n')) * 24;
  const int nameMin          = std::max((available * 2 + 4) / 5, std::min(readable, available * 9 / 20));

  struct Column
  {
    int index;
    int width;
    int floor;  // the least it gets while the name column has its minimum
    bool icons;
  };
  // QTreeView makes sizeHintForColumn protected; QAbstractItemView's is public (same virtual)
  const QAbstractItemView* items = view;
  std::vector<Column> columns;
  int iconWidth = 0, desired = 0, floors = 0;
  for (int c = 0; c < header->count(); ++c) {
    if (c == nameColumn || header->isSectionHidden(c)) {
      continue;
    }
    if (dynamic_cast<IconDelegate*>(view->itemDelegateForColumn(c))) {
      // what the icons need (IconDelegate::sizeHint), not the header's text
      const int w = std::max({items->sizeHintForColumn(c), 24, least});
      columns.push_back({c, w, w, true});
      iconWidth += w;
    } else {
      // ResizeToContents: the contents or the header's text, whichever is wider
      // and, when they must share, at least their contents (FE:000) or a few letters
      const int content = items->sizeHintForColumn(c);
      const int w       = std::max(content, header->sectionSizeHint(c));
      const int floor   = std::max(least, std::min(w, std::max(content, metrics.horizontalAdvance(QStringLiteral("Www")) + 12)));
      columns.push_back({c, w, floor, false});
      desired += w;
      floors += floor;
    }
  }
  // the text columns get what the name column's minimum and the icons leave: their full
  // width, or a share above their floor, or (in a very narrow view) a share of the floors
  const int room = std::max(available - nameMin - iconWidth, 0);
  for (auto& col : columns) {
    if (col.icons || desired <= room) {
      continue;
    }
    if (floors <= room) {
      col.width = col.floor + (col.width - col.floor) * (room - floors) / std::max(desired - floors, 1);
    } else {
      col.width = std::max(least, col.floor * room / std::max(floors, 1));
    }
  }
  // the header's minimum section size may have raised some: take the excess from the widest
  // text columns, so the name column keeps its minimum
  int total = 0;
  for (const auto& col : columns) {
    total += std::max(col.width, least);
  }
  for (int excess = total - (available - nameMin); excess > 0;) {
    auto widest = std::max_element(columns.begin(), columns.end(), [](const Column& a, const Column& b) {
      return (a.icons ? 0 : a.width) < (b.icons ? 0 : b.width);
    });
    if (widest == columns.end() || widest->icons || widest->width <= least) {
      break;
    }
    const int cut = std::min(excess, widest->width - least);
    widest->width -= cut;
    excess -= cut;
  }
  // a stretched last section ignores resizeSection: off while sizing, on again afterwards
  // (the sizes add up to the viewport, so it gets nothing extra)
  const bool stretch = header->stretchLastSection();
  header->setStretchLastSection(false);
  int used = 0;
  for (const auto& col : columns) {
    header->resizeSection(col.index, col.width);
    used += header->sectionSize(col.index);  // what the header really gave (its minimum)
  }
  header->resizeSection(nameColumn, std::max(available - used, least));
  header->setStretchLastSection(stretch);
}

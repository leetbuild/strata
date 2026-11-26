#include "filterpanel.h"

#include <QComboBox>
#include <QEvent>
#include <QHeaderView>
#include <QIcon>
#include <QMouseEvent>
#include <QPushButton>
#include <QRadioButton>
#include <QTreeWidget>

namespace
{

constexpr int KindRole  = Qt::UserRole + 1;
constexpr int IdRole    = Qt::UserRole + 2;
constexpr int StateRole = Qt::UserRole + 3;  // 0 off, 1 on, 2 inverted

QString qs(const rust::String& s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

// CriteriaItem::updateState
QIcon stateIcon(int state)
{
  switch (state) {
  case 1:
    return QIcon(":/MO/gui/checked-checkbox");
  case 2:
    return QIcon(":/MO/gui/indeterminate-checkbox");
  default:
    return QIcon(":/MO/gui/unchecked-checkbox");
  }
}

}  // namespace

FilterPanel::FilterPanel(QTreeWidget* tree, QRadioButton* andButton, QRadioButton* orButton,
                         QComboBox* separators, QPushButton* clear, QObject* parent)
    : QObject(parent), m_tree(tree), m_and(andButton), m_separators(separators)
{
  // FilterList::FilterList
  m_tree->setColumnCount(2);
  m_tree->setHeaderHidden(true);
  m_tree->setRootIsDecorated(false);
  m_tree->header()->setMinimumSectionSize(0);
  m_tree->header()->resizeSection(0, 23);
  m_tree->header()->setStretchLastSection(true);
  m_tree->viewport()->installEventFilter(this);
  m_separators->clear();
  m_separators->addItem(tr("Filter separators"));
  m_separators->addItem(tr("Show separators"));
  m_separators->addItem(tr("Hide separators"));
  if (!andButton->isChecked() && !orButton->isChecked()) {
    andButton->setChecked(true);
  }
  connect(andButton, &QRadioButton::toggled, this, &FilterPanel::changed);
  connect(m_separators, &QComboBox::currentIndexChanged, this, &FilterPanel::changed);
  connect(clear, &QPushButton::clicked, this, [this] {
    for (int i = 0; i < m_tree->topLevelItemCount(); ++i) {
      auto* item = m_tree->topLevelItem(i);
      item->setData(0, StateRole, 0);
      item->setIcon(0, stateIcon(0));
    }
    emit changed();
  });
}

void FilterPanel::setEntries(const rust::Vec<strata::FilterEntry>& entries)
{
  QHash<QPair<int, int>, int> before;
  for (int i = 0; i < m_tree->topLevelItemCount(); ++i) {
    const auto* item = m_tree->topLevelItem(i);
    if (const int state = item->data(0, StateRole).toInt()) {
      before.insert({item->data(0, KindRole).toInt(), item->data(0, IdRole).toInt()}, state);
    }
  }
  m_tree->clear();
  for (const auto& e : entries) {
    auto* item = new QTreeWidgetItem({QString(), qs(e.name)});
    item->setToolTip(1, qs(e.name));
    item->setData(0, KindRole, int(e.kind));
    item->setData(0, IdRole, e.id);
    const int state = before.value({int(e.kind), e.id}, 0);
    item->setData(0, StateRole, state);
    item->setIcon(0, stateIcon(state));
    m_tree->addTopLevelItem(item);
  }
}

rust::Vec<strata::Criterion> FilterPanel::criteria() const
{
  rust::Vec<strata::Criterion> out;
  for (int i = 0; i < m_tree->topLevelItemCount(); ++i) {
    const auto* item = m_tree->topLevelItem(i);
    const int state  = item->data(0, StateRole).toInt();
    if (state == 0) {
      continue;
    }
    strata::Criterion c;
    c.kind    = item->data(0, KindRole).toInt() == 1 ? 1 : 0;
    c.id      = item->data(0, IdRole).toInt();
    c.inverse = state == 2;
    out.push_back(c);
  }
  return out;
}

bool FilterPanel::isAnd() const
{
  return m_and->isChecked();
}

std::uint8_t FilterPanel::separators() const
{
  return std::uint8_t(std::max(0, m_separators->currentIndex()));
}

bool FilterPanel::active() const
{
  return !criteria().empty() || separators() != 0;
}

QStringList FilterPanel::activeNames() const
{
  QStringList names;
  for (int i = 0; i < m_tree->topLevelItemCount(); ++i) {
    const auto* item = m_tree->topLevelItem(i);
    if (const int state = item->data(0, StateRole).toInt()) {
      names << (state == 2 ? tr("not %1").arg(item->text(1)) : item->text(1));
    }
  }
  return names;
}

void FilterPanel::tick(const QString& name)
{
  const auto found = m_tree->findItems(name, Qt::MatchExactly, 1);
  if (!found.isEmpty()) {
    cycle(found.first(), 1);
  }
}

void FilterPanel::cycle(QTreeWidgetItem* item, int step)
{
  const int state = (item->data(0, StateRole).toInt() + step + 3) % 3;
  item->setData(0, StateRole, state);
  item->setIcon(0, stateIcon(state));
  emit changed();
}

bool FilterPanel::eventFilter(QObject* watched, QEvent* event)
{
  // CriteriaItemFilter: a click changes the state instead of selecting
  if (event->type() == QEvent::MouseButtonPress || event->type() == QEvent::MouseButtonDblClick) {
    auto* mouse = static_cast<QMouseEvent*>(event);
    if (auto* item = m_tree->itemAt(mouse->position().toPoint())) {
      cycle(item, mouse->button() == Qt::RightButton ? -1 : 1);
    }
    return true;
  }
  return QObject::eventFilter(watched, event);
}

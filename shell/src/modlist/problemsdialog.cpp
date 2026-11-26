#include "problemsdialog.h"

#include "common.h"
#include "strata_bridge/bridge_builtins.h"
#include "ui_problemsdialog.h"

#include <QCoreApplication>
#include <QDesktopServices>
#include <QMessageBox>
#include <QPointer>
#include <QPushButton>
#include <QThread>

namespace strata::modlist
{

ProblemsDialog::ProblemsDialog(const QString& root, const QString& profile, QWidget* parent)
    : QDialog(parent), ui(new Ui::ProblemsDialog), m_root(root), m_profile(profile)
{
  ui->setupUi(this);
  connect(ui->problemsWidget, &QTreeWidget::itemSelectionChanged, this, [this] {
    QTreeWidgetItem* item = ui->problemsWidget->currentItem();
    const QString text    = item ? item->data(0, Qt::UserRole).toString() : QString();
    ui->descriptionText->setHtml(text);
  });
  // MO2 wraps descriptions without line breaks (selectionChanged); ours have none
  ui->descriptionText->setLineWrapMode(QTextEdit::WidgetWidth);
  ui->descriptionText->setOpenLinks(false);
  connect(ui->descriptionText, &QTextBrowser::anchorClicked, this,
          [](const QUrl& url) { QDesktopServices::openUrl(url); });
  connect(ui->closeButton, &QPushButton::clicked, this, &QDialog::accept);
  runDiagnosis();
}

ProblemsDialog::~ProblemsDialog() = default;

// ProblemsDialog::runDiagnosis, on a worker: the checks read the whole merged tree.
void ProblemsDialog::runDiagnosis()
{
  ui->problemsWidget->clear();
  auto* checking = new QTreeWidgetItem;
  checking->setText(0, tr("Checking..."));
  ui->problemsWidget->addTopLevelItem(checking);

  QPointer<ProblemsDialog> self(this);
  const QString root = m_root, profile = m_profile;
  QThread* worker = QThread::create([self, root, profile] {
    std::vector<Row> rows;
    QString error;
    try {
      for (const auto& p : strata::builtins::problems_collect(rs(root), rs(profile))) {
        rows.push_back({qs(p.id), qs(p.summary), qs(p.description), qs(p.fix)});
      }
    } catch (const std::exception& e) {
      error = QString::fromUtf8(e.what());
    }
    QMetaObject::invokeMethod(QCoreApplication::instance(), [self, rows, error] {
      if (self) {
        self->showRows(rows, error);
      }
    });
  });
  connect(worker, &QThread::finished, worker, &QObject::deleteLater);
  worker->start();
}

void ProblemsDialog::showRows(const std::vector<Row>& rows, const QString& error)
{
  ui->problemsWidget->clear();
  for (const Row& row : rows) {
    auto* item = new QTreeWidgetItem;
    item->setText(0, row.summary);
    item->setData(0, Qt::UserRole, row.description);
    ui->problemsWidget->addTopLevelItem(item);
    if (row.fix.isEmpty()) {
      item->setText(1, tr("No guided fix"));
    } else {
      auto* button = new QPushButton(tr("Fix"));
      button->setToolTip(row.fix);
      const QString id = row.id;
      connect(button, &QPushButton::clicked, this, [this, id] { fix(id); });
      ui->problemsWidget->setItemWidget(item, 1, button);
    }
  }
  if (rows.empty()) {
    auto* item = new QTreeWidgetItem;
    item->setText(0, error.isEmpty() ? tr("(There are no notifications)") : error);
    item->setData(0, Qt::UserRole, QString());
    QFont font = item->font(0);
    font.setItalic(true);
    item->setFont(0, font);
    ui->problemsWidget->addTopLevelItem(item);
  }
  if (!rows.empty()) {
    ui->problemsWidget->setCurrentItem(ui->problemsWidget->topLevelItem(0));
  }
  emit diagnosed(int(rows.size()));
}

// ProblemsDialog::startFix: the fix, then the checks again.
void ProblemsDialog::fix(const QString& id)
{
  try {
    strata::builtins::problem_fix(rs(m_root), rs(m_profile), rs(id));
    m_changed = true;
  } catch (const std::exception& e) {
    QMessageBox::warning(this, tr("Fix"), QString::fromUtf8(e.what()));
  }
  runDiagnosis();
}

}  // namespace strata::modlist

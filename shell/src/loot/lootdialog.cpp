#include "lootdialog.h"

#include "loot/loothost_api.h"
#include "lootreport.h"
#include "strata_bridge/bridge.h"
#include "strata_bridge/bridge_loot.h"
#include "ui_lootdialog.h"

#include <QCloseEvent>
#include <QCoreApplication>
#include <QDataStream>
#include <QDesktopServices>
#include <QDir>
#include <QFile>
#include <QJsonDocument>
#include <QPointer>
#include <QPushButton>
#include <QRegularExpression>
#include <QScrollBar>
#include <QUrl>

#include <atomic>
#include <optional>
#include <thread>

namespace
{

// lootcli::Progress, in its order (strata_loot::Progress has the same numbers).
enum Progress
{
  CheckingMasterlistExistence,
  UpdatingMasterlist,
  LoadingLists,
  ReadingPlugins,
  SortingPlugins,
  WritingLoadorder,
  ParsingLootMessages,
  Done,
};

QString progressToString(int p)
{
  switch (p) {
  case CheckingMasterlistExistence:
    return QObject::tr("Checking masterlist existence");
  case UpdatingMasterlist:
    return QObject::tr("Updating masterlist");
  case LoadingLists:
    return QObject::tr("Loading lists");
  case ReadingPlugins:
    return QObject::tr("Reading plugins");
  case SortingPlugins:
    return QObject::tr("Sorting plugins");
  case WritingLoadorder:
    return QObject::tr("Writing loadorder.txt");
  case ParsingLootMessages:
    return QObject::tr("Parsing loot messages");
  case Done:
    return QObject::tr("Done");
  default:
    return QString("unknown progress %1").arg(p);
  }
}

// log::levelToString
QString levelToString(int level)
{
  switch (level) {
  case 0:
    return "debug";
  case 1:
    return "info";
  case 2:
    return "warning";
  default:
    return "error";
  }
}

// MO2's Loot::m_report file (QDir::temp()/lootreport.json), for "Open JSON report".
QString reportPath()
{
  return QDir::temp().absoluteFilePath("strata-lootreport.json");
}

// [Geometry] LootDialog_details_state: MO2's ExpanderWidget::saveState (a QDataStream bool).
const char* ExpanderKey = "LootDialog_details_state";

}  // namespace

// The core's view of the dialog: called on the sort's worker thread (and libloot's), it posts
// everything to the UI thread. Owned by the worker and the dialog, so it outlives both uses.
class LootRun final : public strata::LootHost
{
public:
  explicit LootRun(LootDialog* dialog) : m_dialog(dialog) {}

  void progress(std::uint8_t step) const override
  {
    QPointer<LootDialog> d = m_dialog;
    QMetaObject::invokeMethod(
        qApp, [d, step] {
          if (d) {
            d->setProgress(step);
          }
        },
        Qt::QueuedConnection);
  }

  void log(std::uint8_t level, rust::Str line) const override
  {
    QPointer<LootDialog> d = m_dialog;
    const QString text     = QString::fromUtf8(line.data(), qsizetype(line.size()));
    QMetaObject::invokeMethod(
        qApp, [d, level, text] {
          if (d) {
            d->log(level, text);
          }
        },
        Qt::QueuedConnection);
  }

  bool cancelled() const override { return m_cancel.load(); }

  std::atomic<bool> m_cancel{false};

private:
  QPointer<LootDialog> m_dialog;
};

LootDialog::LootDialog(QWidget* parent, const QString& root, const QString& profile,
                       bool updateMasterlist)
    : QDialog(parent, Qt::WindowMaximizeButtonHint), ui(new Ui::LootDialog), m_root(root),
      m_profile(profile), m_updateMasterlist(updateMasterlist)
{
  createUI();
}

LootDialog::~LootDialog()
{
  if (m_run) {
    m_run->m_cancel = true;  // a sort still running stops at its next step
  }
  QFile::remove(reportPath());  // Loot::~Loot, deleteReportFile
}

void LootDialog::createUI()
{
  ui->setupUi(this);
  ui->progressBar->setMaximum(0);

  ui->report->setOpenLinks(false);
  connect(ui->report, &QTextBrowser::anchorClicked, this,
          [](const QUrl& url) { QDesktopServices::openUrl(url); });  // MarkdownPage

  // MOBase::ExpanderWidget on the Details button
  ui->details->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
  connect(ui->details, &QToolButton::clicked, this, [this] { setExpanded(!m_expanded); });
  setExpanded(false);

  ui->openJsonReport->setEnabled(false);
  connect(ui->openJsonReport, &QPushButton::clicked, this, [this] { openReport(); });

  ui->buttons->setStandardButtons(QDialogButtonBox::Cancel);

  m_markdown = tr("Running LOOT...");
  ui->report->setMarkdown(m_markdown);

  // always this size, the one that fits the report on a 2560x1440 screen; MO2 restores the last size (GeometrySaver), Strata doesn't for LOOT
  resize(1673, 812);
  setSizeGripEnabled(true);
}

void LootDialog::setExpanded(bool open)
{
  m_expanded = open;
  ui->details->setArrowType(open ? Qt::DownArrow : Qt::RightArrow);
  ui->detailsPanel->setVisible(open);
}

int LootDialog::exec()
{
  std::optional<rust::Box<strata::Session>> session;
  try {
    session.emplace(strata::open_session(m_root.toStdString(), ""));
    const auto v = (*session)->load_setting("Geometry", ExpanderKey);
    if (v.found) {
      QByteArray bytes(reinterpret_cast<const char*>(v.bytes.data()), qsizetype(v.bytes.size()));
      QDataStream stream(bytes);
      bool open = false;
      stream >> open;
      if (stream.status() == QDataStream::Ok) {
        setExpanded(open);
      }
    }
  } catch (const std::exception& e) {
    qWarning("loot dialog: %s", e.what());
  }

  if (!m_started) {
    m_started = true;
    // Loot::start, lootThread
    m_run                     = std::make_shared<LootRun>(this);
    auto run                  = m_run;
    const std::string root    = m_root.toStdString();
    const std::string profile = m_profile.toStdString();
    const bool update         = m_updateMasterlist;
    QPointer<LootDialog> self(this);
    std::thread([run, root, profile, update, self] {
      QString json, error;
      try {
        json = QString::fromStdString(
            std::string(strata::sort_plugins_report(root, profile, update, *run)));
      } catch (const std::exception& e) {
        error = QString::fromUtf8(e.what());
        if (error.isEmpty()) {
          error = "unknown error";
        }
      }
      QMetaObject::invokeMethod(
          qApp, [self, json, error] {
            if (self) {
              self->onFinished(json, error);
            }
          },
          Qt::QueuedConnection);
    }).detach();
  }

  const int r = QDialog::exec();

  if (session) {
    try {
      QByteArray bytes;
      QDataStream stream(&bytes, QIODevice::WriteOnly);
      stream << m_expanded;
      strata::SettingValue v;
      v.section = "Geometry";
      v.key     = ExpanderKey;
      for (char c : bytes) {
        v.bytes.push_back(std::uint8_t(c));
      }
      v.kind = 1;
      rust::Vec<strata::SettingValue> values;
      values.push_back(std::move(v));
      (*session)->store_settings(std::move(values));
    } catch (const std::exception& e) {
      qWarning("loot dialog: %s", e.what());
    }
  }
  return r;
}

void LootDialog::setText(const QString& s)
{
  ui->progressText->setText(s);
}

void LootDialog::setProgress(int step)
{
  // don't overwrite the "stopping loot" message
  if (!m_cancelling) {
    setText(progressToString(step));
  }
  if (step == Done) {
    ui->progressBar->setRange(0, 1);
    ui->progressBar->setValue(1);
  }
}

void LootDialog::log(int level, const QString& line)
{
  // Loot::processMessage keeps errors and warnings for the report
  if (level >= 3) {
    m_errors << line;
    qWarning("loot: %s", qUtf8Printable(line));
  } else if (level == 2) {
    m_warnings << line;
    qWarning("loot: %s", qUtf8Printable(line));
  }
  addLineOutput(QString("[%1] %2").arg(levelToString(level), line));
}

void LootDialog::addLineOutput(const QString& line)
{
  ui->output->appendPlainText(line);
}

void LootDialog::cancel()
{
  if (!m_finished && !m_cancelling) {
    if (m_run) {
      m_run->m_cancel = true;
    }
    setText(tr("Stopping LOOT..."));
    addLineOutput("stopping loot");
    ui->buttons->setEnabled(false);
    m_cancelling = true;
  }
}

void LootDialog::accept()
{
  // no-op (MO2: the Close button is a reject)
}

void LootDialog::reject()
{
  if (m_finished || !m_started) {
    QDialog::reject();
  } else {
    cancel();
  }
}

void LootDialog::closeEvent(QCloseEvent* e)
{
  if (m_finished || !m_started) {
    QDialog::closeEvent(e);
  } else {
    cancel();
    e->ignore();
  }
}

void LootDialog::onFinished(const QString& json, const QString& error)
{
  m_finished = true;
  if (m_cancelling) {
    close();
    return;
  }
  if (error.isEmpty()) {
    QJsonParseError e;
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &e);
    if (doc.isObject()) {
      m_succeeded  = true;
      m_report     = doc.object();
    } else {
      m_errors << QString("invalid json, %1 (error %2)").arg(e.errorString()).arg(e.error);
    }
  } else {
    m_errors << error;
  }
  showReport();
  ui->openJsonReport->setEnabled(m_succeeded);
  ui->buttons->setEnabled(true);
  ui->buttons->setStandardButtons(QDialogButtonBox::Close);
  // a failed run sends no Done: stop the bar
  setProgress(Done);
}

void LootDialog::showReport()
{
  m_markdown = loot::reportMarkdown(m_succeeded, m_errors, m_warnings, m_report);
  ui->report->setMarkdown(m_markdown);
}

void LootDialog::openReport()
{
  QFile f(reportPath());
  if (!f.open(QIODevice::WriteOnly | QIODevice::Truncate)) {
    qWarning("cannot write %s: %s", qUtf8Printable(reportPath()), qUtf8Printable(f.errorString()));
    return;
  }
  f.write(QJsonDocument(m_report).toJson(QJsonDocument::Indented));
  f.close();
  QDesktopServices::openUrl(QUrl::fromLocalFile(reportPath()));
}

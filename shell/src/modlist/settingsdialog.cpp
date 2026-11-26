#include "settingsdialog.h"

#include "common.h"
#include "strata_bridge/bridge.h"
#include "strata_bridge/bridge_install.h"
#include "strata_bridge/bridge_modlist.h"
#include "ui_nexusmanualkey.h"
#include "ui_settingsdialog.h"

#include <QApplication>
#include <QClipboard>
#include <QFontDatabase>
#include <QJsonDocument>
#include <QJsonObject>
#include <QThread>

#include <QCheckBox>
#include <QComboBox>
#include <QCoreApplication>
#include <QDesktopServices>
#include <QDir>
#include <QFileDialog>
#include <QFormLayout>
#include <QGroupBox>
#include <QHeaderView>
#include <QInputDialog>
#include <QLabel>
#include <QLineEdit>
#include <QMessageBox>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QSpinBox>
#include <QUrl>
#include <QVBoxLayout>

namespace strata::modlist
{

namespace
{

void unported(QWidget* w, const QString& why)
{
  if (w) {
    w->setEnabled(false);
    w->setToolTip(why);
  }
}

}  // namespace

SettingsDialog::SettingsDialog(const QString& root, QWidget* parent)
    : QDialog(parent), ui(new Ui::SettingsDialog), m_root(root)
{
  ui->setupUi(this);
  addLeetTab();
  fillChoices();
  disableUnported();
  load();
  setupNexus();

  // the Paths tab's browse buttons
  const std::pair<QPushButton*, QLineEdit*> browse[] = {
      {ui->browseBaseDirBtn, ui->baseDirEdit},         {ui->browseDownloadDirBtn, ui->downloadDirEdit},
      {ui->browseModDirBtn, ui->modDirEdit},           {ui->browseCacheDirBtn, ui->cacheDirEdit},
      {ui->browseProfilesDirBtn, ui->profilesDirEdit}, {ui->browseOverwriteDirBtn, ui->overwriteDirEdit}};
  for (const auto& [button, edit] : browse) {
    connect(button, &QPushButton::clicked, this, [this, edit = edit] {
      const QString dir = QFileDialog::getExistingDirectory(this, tr("Select a folder"), edit->text());
      if (!dir.isEmpty()) {
        edit->setText(dir);
      }
    });
  }
  connect(ui->execBlacklistBtn, &QPushButton::clicked, this,
          [this] { editList("execBlacklistBtn", tr("Executables Blacklist"), ";"); });
  connect(ui->skipFileSuffixBtn, &QPushButton::clicked, this,
          [this] { editList("skipFileSuffixBtn", tr("Skip File Suffixes"), {}); });
  connect(ui->skipDirectoriesBtn, &QPushButton::clicked, this,
          [this] { editList("skipDirectoriesBtn", tr("Skip Directories"), {}); });
  connect(ui->resetDialogsButton, &QPushButton::clicked, this, [this] {
    if (QMessageBox::question(this, tr("Confirm?"),
                              tr("This will reset all the choices you made to dialogs and make "
                                 "them all visible again. Continue?")) == QMessageBox::Yes) {
      try {
        reset_dialog_choices(utf8(m_root));
        m_changed = true;
      } catch (const std::exception& e) {
        QMessageBox::warning(this, windowTitle(), QString::fromUtf8(e.what()));
      }
    }
  });
  connect(ui->exploreStyles, &QPushButton::clicked, this, [this] {
    QDesktopServices::openUrl(QUrl::fromLocalFile(m_root + "/stylesheets"));
  });
  connect(ui->useCustomBrowser, &QCheckBox::toggled, ui->browserCommand, &QWidget::setEnabled);
  connect(ui->browseCustomBrowser, &QPushButton::clicked, this, [this] {
    const QString file = QFileDialog::getOpenFileName(this, tr("Select the browser executable"));
    if (!file.isEmpty()) {
      ui->browserCommand->setText(file);
    }
  });
  ui->managedGameDirEdit->setText(qs(game_folder(utf8(m_root))));

  // the form connects its button box to accept / reject; OK stores first
  disconnect(ui->buttonBox, &QDialogButtonBox::accepted, this, nullptr);
  connect(ui->buttonBox, &QDialogButtonBox::accepted, this, [this] {
    if (store()) {
      accept();
    }
  });
}

SettingsDialog::~SettingsDialog()
{
  // a login waits for the browser without end: stop it, then let the worker finish
  if (m_nexusWorker) {
    nexus_cancel_login();
    m_nexusWorker->wait();
  }
}

namespace
{

// MO2's NexusManualKeyDialog (nexusmanualkey.ui)
class NexusManualKeyDialog : public QDialog
{
public:
  explicit NexusManualKeyDialog(QWidget* parent) : QDialog(parent), ui(new Ui::NexusManualKeyDialog)
  {
    ui->setupUi(this);
    ui->key->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
    connect(ui->openBrowser, &QPushButton::clicked, this,
            [] { QDesktopServices::openUrl(QUrl("https://www.nexusmods.com/users/myaccount?tab=api")); });
    connect(ui->paste, &QPushButton::clicked, this, [this] {
      const QString text = QApplication::clipboard()->text();
      if (!text.isEmpty()) {
        ui->key->setPlainText(text);
      }
    });
    connect(ui->clear, &QPushButton::clicked, ui->key, &QPlainTextEdit::clear);
  }
  QString key() const { return ui->key->toPlainText().trimmed(); }

private:
  std::unique_ptr<Ui::NexusManualKeyDialog> ui;
};

QString fromRust(const rust::String& s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

}  // namespace

void SettingsDialog::setupNexus()
{
  connect(ui->nexusConnect, &QPushButton::clicked, this, &SettingsDialog::nexusConnect);
  connect(ui->nexusManualKey, &QPushButton::clicked, this, &SettingsDialog::nexusManual);
  connect(ui->nexusDisconnect, &QPushButton::clicked, this, &SettingsDialog::nexusDisconnect);
  connect(ui->associateButton, &QPushButton::clicked, this, [this] {
    try {
      nexusLog(fromRust(nexus_associate()));
    } catch (const std::exception& e) {
      QMessageBox::warning(this, windowTitle(), QString::fromUtf8(e.what()));
    }
  });
  connect(ui->clearCacheButton, &QPushButton::clicked, this, [this] {
    try {
      nexusLog(fromRust(clear_web_cache(utf8(m_root))));
    } catch (const std::exception& e) {
      QMessageBox::warning(this, windowTitle(), QString::fromUtf8(e.what()));
    }
  });
  ui->associateButton->setToolTip(tr("Make Strata the handler of nxm:// links (\"Mod Manager Download\" on Nexus)."));

  m_hasKey = nexus_has_key();
  nexusShowAccount({});
  nexusLog(m_hasKey ? tr("Connected.") : tr("Not connected."));
  nexusUpdateState();
  if (m_hasKey) {
    // MO2 shows the account it validated at start; Strata asks when the dialog opens
    nexusRun(
        NexusBusy::Check, [] { return fromRust(nexus_account()); },
        [this](const QString& r, const QString& error) {
          if (!error.isEmpty()) {
            nexusLog(error);
          }
          nexusShowAccount(r);
        });
  }
}

void SettingsDialog::state(rust::Str line, rust::Str url)
{
  const QString text = QString::fromUtf8(line.data(), qsizetype(line.size()));
  const QString page = QString::fromUtf8(url.data(), qsizetype(url.size()));
  QMetaObject::invokeMethod(this, [this, text, page] {
    nexusLog(text);
    if (!page.isEmpty()) {
      QDesktopServices::openUrl(QUrl(page));
    }
  });
}

void SettingsDialog::nexusRun(NexusBusy busy, std::function<QString()> work,
                              std::function<void(QString, QString)> done)
{
  auto result = std::make_shared<std::pair<QString, QString>>();
  m_nexusBusy = busy;
  m_nexusWorker = QThread::create([work = std::move(work), result] {
    try {
      result->first = work();
    } catch (const std::exception& e) {
      result->second = QString::fromUtf8(e.what());
    }
  });
  m_nexusWorker->setParent(this);
  connect(m_nexusWorker, &QThread::finished, this, [this, result, done = std::move(done)] {
    m_nexusWorker->deleteLater();
    m_nexusWorker = nullptr;
    m_nexusBusy = NexusBusy::None;
    done(result->first, result->second);
    nexusUpdateState();
  });
  m_nexusWorker->start();
  nexusUpdateState();
}

// NexusConnectionUI::connect: a second click while waiting cancels
void SettingsDialog::nexusConnect()
{
  if (m_nexusBusy == NexusBusy::Login) {
    nexus_cancel_login();
    return;
  }
  ui->nexusLog->clear();
  nexusRun(
      NexusBusy::Login, [this] { return fromRust(nexus_login_sso(*this)); },
      [this](const QString& r, const QString& error) {
        if (!error.isEmpty()) {
          nexusLog(error);
          return;
        }
        m_hasKey = true;
        nexusLog(tr("Linked with Nexus successfully."));
        nexusShowAccount(r);
      });
}

void SettingsDialog::nexusManual()
{
  NexusManualKeyDialog d(this);
  if (d.exec() != QDialog::Accepted) {
    return;
  }
  const QString key = d.key();
  if (key.isEmpty()) {
    nexusDisconnect();
    return;
  }
  ui->nexusLog->clear();
  nexusLog(tr("Checking API key..."));
  const std::string k = key.toStdString();
  nexusRun(
      NexusBusy::Check, [k] { return fromRust(nexus_set_key(k)); },
      [this](const QString& r, const QString& error) {
        if (!error.isEmpty()) {
          nexusLog(error);
          return;
        }
        m_hasKey = true;
        nexusLog(tr("Linked with Nexus successfully."));
        nexusShowAccount(r);
      });
}

void SettingsDialog::nexusDisconnect()
{
  ui->nexusLog->clear();
  try {
    nexus_logout();
    m_hasKey = false;
    nexusShowAccount({});
    nexusLog(tr("Disconnected."));
  } catch (const std::exception& e) {
    nexusLog(QString::fromUtf8(e.what()));
  }
  nexusUpdateState();
}

// NexusConnectionUI::updateState
void SettingsDialog::nexusUpdateState()
{
  auto set = [](QPushButton* b, bool enabled, const QString& caption = {}) {
    b->setEnabled(enabled);
    if (!caption.isEmpty()) {
      b->setText(caption);
    }
  };
  const QString connectText = tr("Connect to Nexus");
  const QString manualText  = tr("Enter API Key Manually");
  if (m_nexusBusy == NexusBusy::Login) {
    set(ui->nexusConnect, true, tr("Cancel"));
    set(ui->nexusDisconnect, false);
    set(ui->nexusManualKey, false, manualText);
  } else if (m_nexusBusy == NexusBusy::Check) {
    set(ui->nexusConnect, false, connectText);
    set(ui->nexusDisconnect, false);
    set(ui->nexusManualKey, false, manualText);
  } else if (m_hasKey) {
    set(ui->nexusConnect, false, connectText);
    set(ui->nexusDisconnect, true);
    set(ui->nexusManualKey, false, manualText);
  } else {
    set(ui->nexusConnect, true, connectText);
    set(ui->nexusDisconnect, false);
    set(ui->nexusManualKey, true, manualText);
  }
}

// NexusSettingsTab::updateNexusData; `json` is strata_ui::nexus::Account, empty for none
void SettingsDialog::nexusShowAccount(const QString& json)
{
  const auto a = QJsonDocument::fromJson(json.toUtf8()).object();
  if (a.isEmpty()) {
    for (QLabel* l : {ui->nexusUserID, ui->nexusName, ui->nexusAccount, ui->nexusDailyRequests,
                      ui->nexusHourlyRequests}) {
      l->setText(tr("N/A"));
    }
    return;
  }
  auto ratio = [&](const char* remaining, const char* limit) {
    return a[remaining].isNull() ? tr("N/A")
                                 : QString("%1/%2").arg(a[remaining].toInt()).arg(a[limit].toInt());
  };
  ui->nexusUserID->setText(QString::number(qint64(a["id"].toDouble())));
  ui->nexusName->setText(a["name"].toString());
  ui->nexusAccount->setText(a["kind"].toString());
  ui->nexusDailyRequests->setText(ratio("daily_remaining", "daily_limit"));
  ui->nexusHourlyRequests->setText(ratio("hourly_remaining", "hourly_limit"));
}

void SettingsDialog::nexusLog(const QString& lines)
{
  for (const QString& line : lines.split('\n')) {
    ui->nexusLog->addItem(line);
  }
  ui->nexusLog->scrollToBottom();
}

// Leet's settings: auto-enable after install (change 11), the overwrite check off by
// default (change 8), Sub-Separator (change 9; the Leet backlog's missing settings page)
void SettingsDialog::addLeetTab()
{
  auto* page   = new QWidget(this);
  auto* layout = new QVBoxLayout(page);

  auto* general = new QGroupBox(tr("Leet Edition"), page);
  auto* g       = new QVBoxLayout(general);
  auto* autoEnable = new QCheckBox(tr("Enable mods after installing them"), general);
  autoEnable->setObjectName("leetAutoEnable");
  auto* checkOverwrite = new QCheckBox(tr("Warn when overwrite is not empty"), general);
  checkOverwrite->setObjectName("leetCheckOverwrite");
  auto* checkFont = new QCheckBox(tr("Warn when the font configuration refers to fonts that aren't installed"), general);
  checkFont->setObjectName("leetCheckFont");
  auto* checkAlternate = new QCheckBox(tr("Warn when an active mod came from another game's source"), general);
  checkAlternate->setObjectName("leetCheckAlternateGames");
  g->addWidget(autoEnable);
  g->addWidget(checkOverwrite);
  g->addWidget(checkFont);
  g->addWidget(checkAlternate);
  layout->addWidget(general);

  // third-party MO2 plugins, built in: each can be switched off ([PluginPersistance])
  auto* builtins = new QGroupBox(tr("Built-in plugins"), page);
  auto* b        = new QVBoxLayout(builtins);
  const std::vector<std::pair<const char*, QString>> boxes{
      {"builtinCrashLogLabeler", tr("Crash Log Labeler (labels new crash logs after a run)")},
      {"builtinCrashLogOffline", tr("Crash Log Labeler: disable update from remote database")},
      {"builtinCrashLogViewer", tr("Crash Log Viewer (Tools menu)")},
  };
  for (const auto& [name, text] : boxes) {
    auto* box = new QCheckBox(text, builtins);
    box->setObjectName(name);
    b->addWidget(box);
  }
  layout->addWidget(builtins);

  // Strata's own: environment variables for every launch ([Settings] strata_environment)
  auto* launch  = new QGroupBox(tr("Launching (Proton)"), page);
  auto* lv      = new QVBoxLayout(launch);
  auto* envBtn  = new QPushButton(tr("Environment variables..."), launch);
  envBtn->setObjectName("strataEnvironmentBtn");
  envBtn->setToolTip(tr("Set for every program started from this instance, one NAME=value per line, on top of "
                        "Strata's own (for example PROTON_NO_NTSYNC=1, DXVK_HUD=fps). Lines starting with # are "
                        "ignored."));
  connect(envBtn, &QPushButton::clicked, this,
          [this] { editList("strataEnvironmentBtn", tr("Environment variables (NAME=value)"), {}); });
  lv->addWidget(envBtn, 0, Qt::AlignLeft);
  layout->addWidget(launch);

  auto* sub  = new QGroupBox(tr("Sub-Separator"), page);
  auto* form = new QFormLayout(sub);
  auto* size = new QCheckBox(tr("Always show the number of items"), sub);
  size->setObjectName("leetSubSeparatorAlwaysShowSize");
  auto* indent = new QSpinBox(sub);
  indent->setObjectName("leetSubSeparatorIndentation");
  indent->setRange(-1, 100);
  indent->setSpecialValueText(tr("Qt default"));
  auto* invert = new QCheckBox(tr("Invert Alt+click on collapsed separators"), sub);
  invert->setObjectName("leetSubSeparatorInvertAlt");
  form->addRow(size);
  form->addRow(tr("Indentation"), indent);
  form->addRow(invert);
  layout->addWidget(sub);
  layout->addStretch(1);

  ui->tabWidget->addTab(page, tr("Leet"));
}

void SettingsDialog::fillChoices()
{
  // styles: every sheet in the folders the window searches; "" = the default (riki when found)
  ui->styleBox->clear();
  const QString fallback =
      qs(strata::default_stylesheet(rs(m_root), rs(QCoreApplication::applicationDirPath()))).chopped(4);
  ui->styleBox->addItem(tr("Default (%1)").arg(fallback), QString());
  QStringList seen;
  for (const auto& dir : strata::stylesheet_dirs(rs(m_root), rs(QCoreApplication::applicationDirPath()))) {
    for (const QString& file : QDir(qs(dir)).entryList({"*.qss"}, QDir::Files, QDir::Name)) {
      if (!seen.contains(file, Qt::CaseInsensitive)) {
        seen.push_back(file);
        ui->styleBox->addItem(file.chopped(4), file);
      }
    }
  }

  // DiagnosticsSettingsTab::setLogLevel / setLootLogLevel
  ui->logLevelBox->clear();
  const std::pair<QString, int> levels[] = {{tr("Debug"), 0}, {tr("Info (recommended)"), 1},
                                            {tr("Warning"), 2}, {tr("Error"), 3}};
  for (const auto& [text, value] : levels) {
    ui->logLevelBox->addItem(text, value);
  }
  ui->lootLogLevel->clear();
  const std::pair<QString, int> loot[] = {{tr("Trace"), 0}, {tr("Debug"), 1}, {tr("Info (recommended)"), 2},
                                          {tr("Warning"), 3}, {tr("Error"), 4}};
  for (const auto& [text, value] : loot) {
    ui->lootLogLevel->addItem(text, value);
  }
}

void SettingsDialog::disableUnported()
{
  const QString english = tr("Strata is English only.");
  const QString mo2     = tr("Strata is not updated by MO2's updater.");
  const QString later   = tr("Not in Strata yet.");
  const QString windows = tr("Only meaningful on Windows.");
  // MO2 always offers English (GeneralSettingsTab::addLanguages); Strata has nothing else
  ui->languageBox->addItem(QStringLiteral("English"), QStringLiteral("en_US"));
  unported(ui->languageBox, english);
  unported(ui->checkForUpdates, mo2);
  unported(ui->usePrereleaseBox, mo2);
  unported(ui->categoriesBtn, later);
  unported(ui->resetColorsBtn, later);
  fillColorTable();
  unported(ui->colorTable, tr("Shown as stored; editing the colours is not in Strata yet."));
  unported(ui->managedGameDirEdit, tr("Set when the instance is created."));
  unported(ui->browseGameDirBtn, tr("Set when the instance is created."));
  // MO2 fills these from the download servers it has seen; Strata keeps no such list
  for (QWidget* w : std::initializer_list<QWidget*>{ui->knownServersList, ui->preferredServersList}) {
    unported(w, tr("Not in Strata yet: downloads use the server Nexus picks."));
  }
  // PluginsSettingsTab hides the list's header; Strata lists no plugins
  ui->pluginsList->setHeaderHidden(true);
  ui->noPluginLabel->setText(tr("Strata has no plugins: the MO2 plugins it keeps are built in."));
  ui->tabWidget->setTabEnabled(ui->tabWidget->indexOf(ui->pluginsTab), false);
  ui->tabWidget->setTabToolTip(ui->tabWidget->indexOf(ui->pluginsTab), tr("Strata has no plugins: they are built in."));
  for (QWidget* w : std::initializer_list<QWidget*>{ui->steamUserEdit, ui->steamPassEdit, ui->appIDEdit}) {
    unported(w, tr("Steam is started by Strata itself; no login is stored."));
  }
  unported(ui->dumpsTypeBox, windows);
  unported(ui->resetGeometryBtn, later);
  unported(ui->bsaDateBtn, windows);
}

// ColorTable::load: caption, a sample in the colour (MO2's two icon columns are left out)
void SettingsDialog::fillColorTable()
{
  const auto rows = highlight_colors(utf8(m_root));
  ui->colorTable->setColumnCount(2);
  ui->colorTable->setRowCount(int(rows.size()));
  ui->colorTable->horizontalHeader()->hide();
  ui->colorTable->verticalHeader()->hide();
  for (int r = 0; r < int(rows.size()); ++r) {
    ui->colorTable->setItem(r, 0, new QTableWidgetItem(qs(rows[r].caption)));
    auto* sample = new QTableWidgetItem(tr("Text"));
    sample->setBackground(QColor(qs(rows[r].color)));
    ui->colorTable->setItem(r, 1, sample);
  }
  ui->colorTable->resizeColumnsToContents();
}

void SettingsDialog::load()
{
  try {
    for (const auto& row : setting_rows(utf8(m_root))) {
      const QString name  = qs(row.widget);
      const QString value = qs(row.value);
      QWidget* w          = findChild<QWidget*>(name);
      if (auto* box = qobject_cast<QCheckBox*>(w)) {
        box->setChecked(value == "true");
      } else if (auto* edit = qobject_cast<QLineEdit*>(w)) {
        edit->setText(value);
      } else if (auto* spin = qobject_cast<QSpinBox*>(w)) {
        spin->setValue(value.toInt());
      } else if (auto* combo = qobject_cast<QComboBox*>(w)) {
        const QVariant data = row.kind == 1 ? QVariant(value.toInt()) : QVariant(value);
        int index           = combo->findData(data);
        if (index < 0 && row.kind == 2 && !value.isEmpty()) {
          combo->addItem(value + tr(" (not found)"), value);  // a sheet that isn't there
          index = combo->count() - 1;
        }
        combo->setCurrentIndex(std::max(index, 0));
      } else if (qobject_cast<QPushButton*>(w)) {
        m_lists[name] = value;
      }
    }
  } catch (const std::exception& e) {
    QMessageBox::warning(this, windowTitle(), QString::fromUtf8(e.what()));
  }
  ui->browserCommand->setEnabled(ui->useCustomBrowser->isChecked());

  // DiagnosticsSettingsTab: the label names the instance's logs and crash dumps folders
  const QString logs  = QUrl::fromLocalFile(m_root + "/logs").toString();
  const QString dumps = QUrl::fromLocalFile(m_root + "/crashDumps").toString();
  ui->diagnosticsExplainedLabel->setText(ui->diagnosticsExplainedLabel->text()
                                             .replace("LOGS_FULL_PATH", logs)
                                             .replace("LOGS_DIR", "logs")
                                             .replace("DUMPS_FULL_PATH", dumps)
                                             .replace("DUMPS_DIR", "crashDumps"));
}

void SettingsDialog::editList(const QString& widget, const QString& title, const QString& separator)
{
  QString text = m_lists.value(widget);
  if (!separator.isEmpty()) {
    text = text.split(separator, Qt::SkipEmptyParts).join('\n');
  }
  bool ok            = false;
  const QString edit = QInputDialog::getMultiLineText(this, title, tr("One per line:"), text, &ok);
  if (ok) {
    const QStringList lines = edit.split('\n', Qt::SkipEmptyParts);
    m_lists[widget]         = separator.isEmpty() ? lines.join('\n') : lines.join(separator);
  }
}

bool SettingsDialog::store()
{
  rust::Vec<SettingPair> values;
  try {
    for (const auto& row : setting_rows(utf8(m_root))) {
      const QString name = qs(row.widget);
      QWidget* w         = findChild<QWidget*>(name);
      QString value;
      if (auto* box = qobject_cast<QCheckBox*>(w)) {
        value = box->isChecked() ? "true" : "false";
      } else if (auto* edit = qobject_cast<QLineEdit*>(w)) {
        value = edit->text();
      } else if (auto* spin = qobject_cast<QSpinBox*>(w)) {
        value = QString::number(spin->value());
      } else if (auto* combo = qobject_cast<QComboBox*>(w)) {
        value = combo->currentData().toString();
      } else if (qobject_cast<QPushButton*>(w)) {
        value = m_lists.value(name);
      } else {
        continue;
      }
      SettingPair pair;
      pair.widget = rs(name);
      pair.value  = rs(value);
      values.push_back(std::move(pair));
    }
    m_changed |= settings_store(utf8(m_root), std::move(values));
    return true;
  } catch (const std::exception& e) {
    QMessageBox::warning(this, windowTitle(), QString::fromUtf8(e.what()));
    return false;
  }
}

}  // namespace strata::modlist

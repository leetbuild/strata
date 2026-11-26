#include "instances/instancedialogs.h"

#include "strata_bridge/bridge_instances.h"
#include "ui_instancemanagerdialog.h"

#include <QButtonGroup>
#include <QCheckBox>
#include <QComboBox>
#include <QCoreApplication>
#include <QDesktopServices>
#include <QDialogButtonBox>
#include <functional>
#include <QDir>
#include <QFileDialog>
#include <QFileInfo>
#include <QFormLayout>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMessageBox>
#include <QPlainTextEdit>
#include <QProcess>
#include <QPushButton>
#include <QRadioButton>
#include <QSortFilterProxyModel>
#include <QStackedWidget>
#include <QStandardItemModel>
#include <QUrl>
#include <QVBoxLayout>

namespace strata::instances_ui
{
namespace
{

QString qs(const rust::String& s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

rust::String rs(const QString& s)
{
  return rust::String(s.toStdString());
}

QString what(const std::exception& e)
{
  return QString::fromUtf8(e.what());
}

void explore(const QString& path)
{
  if (!path.isEmpty()) {
    QDesktopServices::openUrl(QUrl::fromLocalFile(path));
  }
}

// The running program, for `strata prefix setup` (Linux names a replaced file
// "<path> (deleted)"; strata_core::dirs::own_exe drops that too).
QString ownProgram()
{
  QString path = QCoreApplication::applicationFilePath();
  if (path.endsWith(" (deleted)")) {
    path.chop(int(qstrlen(" (deleted)")));
  }
  return path;
}

struct Row
{
  QString name, root, base, game, gameDir, prefix, proton;
  bool supported = false;
};

// ---------------------------------------------------------------------------------------
// The new-instance wizard (MO2's CreateInstanceDialog: pages game, name and location,
// then Strata's Proton page, then the review; Back / Next / Create). MO2's form is built
// around global vs portable instances and Windows paths, so the pages are made here.
class CreateInstanceDialog : public QDialog
{
public:
  explicit CreateInstanceDialog(QWidget* parent) : QDialog(parent)
  {
    setObjectName("CreateInstanceDialog");
    setWindowTitle(tr("Create new instance"));
    resize(760, 520);
    for (const auto& g : strata::instances::games()) {
      m_games.push_back({qs(g.name), qs(g.binary), qs(g.steam_dir)});
    }
    for (const auto& p : strata::instances::protons()) {
      m_protons.push_back({qs(p.name), qs(p.path), qs(p.note)});
    }
    m_root = qs(strata::instances::instances_root());

    auto* layout = new QVBoxLayout(this);
    m_title = new QLabel(this);
    QFont big = m_title->font();
    big.setPointSizeF(big.pointSizeF() * 1.3);
    big.setBold(true);
    m_title->setFont(big);
    layout->addWidget(m_title);
    m_pages = new QStackedWidget(this);
    layout->addWidget(m_pages, 1);
    m_pages->addWidget(gamePage());
    m_pages->addWidget(namePage());
    m_pages->addWidget(protonPage());
    m_pages->addWidget(reviewPage());

    auto* buttons = new QHBoxLayout;
    buttons->addStretch(1);
    m_back   = new QPushButton(tr("< Back"), this);
    m_next   = new QPushButton(tr("Next >"), this);
    m_cancel = new QPushButton(tr("Cancel"), this);
    m_back->setObjectName("back");
    m_next->setObjectName("next");
    m_cancel->setObjectName("cancel");
    m_next->setDefault(true);
    buttons->addWidget(m_back);
    buttons->addWidget(m_next);
    buttons->addWidget(m_cancel);
    layout->addLayout(buttons);
    connect(m_back, &QPushButton::clicked, this, [this] { go(m_pages->currentIndex() - 1); });
    connect(m_next, &QPushButton::clicked, this, [this] { next(); });
    connect(m_cancel, &QPushButton::clicked, this, [this] { cancel(); });
    go(0);
  }

  QString created() const { return m_created; }
  bool switching() const { return !m_created.isEmpty() && m_switch->isChecked(); }

private:
  struct Game
  {
    QString name, binary, steamDir;
  };
  struct Proton
  {
    QString name, path, note;
  };

  std::vector<Game> m_games;
  std::vector<Proton> m_protons;
  QString m_root;
  QString m_created;

  QLabel* m_title             = nullptr;
  QStackedWidget* m_pages     = nullptr;
  QPushButton *m_back = nullptr, *m_next = nullptr, *m_cancel = nullptr;
  QListWidget* m_gameList     = nullptr;
  QLineEdit* m_gameDir        = nullptr;
  QLabel* m_gameError         = nullptr;
  QLineEdit* m_name           = nullptr;
  QLineEdit* m_location       = nullptr;
  QLabel* m_nameError         = nullptr;
  bool m_locationEdited       = false;
  QComboBox* m_proton         = nullptr;
  QLabel* m_protonNote        = nullptr;
  QRadioButton* m_newPrefix   = nullptr;
  QRadioButton* m_oldPrefix   = nullptr;
  QLineEdit* m_prefix         = nullptr;
  QLabel* m_prefixError       = nullptr;
  QCheckBox* m_localInis      = nullptr;
  QCheckBox* m_localSaves     = nullptr;
  QPlainTextEdit* m_review    = nullptr;
  QCheckBox* m_setup          = nullptr;
  QCheckBox* m_switch         = nullptr;
  QPlainTextEdit* m_log       = nullptr;
  QProcess* m_process         = nullptr;

  static QLabel* errorLabel(QWidget* parent)
  {
    auto* l = new QLabel(parent);
    l->setWordWrap(true);
    l->setStyleSheet("color: #e05050");  // MO2's createinstancedialog error labels are red
    return l;
  }

  static QHBoxLayout* pathRow(QWidget* parent, QLineEdit* edit, const std::function<void()>& browse)
  {
    auto* row    = new QHBoxLayout;
    auto* button = new QPushButton(QObject::tr("Browse..."), parent);
    row->addWidget(edit, 1);
    row->addWidget(button);
    QObject::connect(button, &QPushButton::clicked, parent, browse);
    return row;
  }

  QWidget* gamePage()
  {
    auto* page = new QWidget(this);
    auto* l    = new QVBoxLayout(page);
    auto* intro = new QLabel(tr("Select the game to manage. Strata looks for it in Steam's libraries; you can "
                                "also point it at a copy of the game folder (a \"Game Root\" kept apart from "
                                "Steam's updates)."),
                             page);
    intro->setWordWrap(true);
    l->addWidget(intro);
    m_gameList = new QListWidget(page);
    m_gameList->setObjectName("games");
    for (const auto& g : m_games) {
      auto* item = new QListWidgetItem(
          g.steamDir.isEmpty() ? tr("%1  (not found in Steam)").arg(g.name) : QStringLiteral("%1  \u2013  %2").arg(g.name, g.steamDir),
          m_gameList);
      item->setData(Qt::UserRole, g.name);
    }
    l->addWidget(m_gameList, 1);
    l->addWidget(new QLabel(tr("Game folder"), page));
    m_gameDir = new QLineEdit(page);
    m_gameDir->setObjectName("gameDir");
    l->addLayout(pathRow(page, m_gameDir, [this] {
      const QString dir = QFileDialog::getExistingDirectory(this, tr("Game folder"), m_gameDir->text());
      if (!dir.isEmpty()) {
        m_gameDir->setText(dir);
      }
    }));
    m_gameError = errorLabel(page);
    l->addWidget(m_gameError);
    connect(m_gameList, &QListWidget::currentRowChanged, this, [this](int row) {
      if (row >= 0 && row < int(m_games.size())) {
        m_gameDir->setText(m_games[std::size_t(row)].steamDir);
      }
      validate();
    });
    connect(m_gameDir, &QLineEdit::textChanged, this, [this] { validate(); });
    if (!m_games.empty()) {
      m_gameList->setCurrentRow(0);
    }
    return page;
  }

  QWidget* namePage()
  {
    auto* page = new QWidget(this);
    auto* l    = new QVBoxLayout(page);
    auto* intro = new QLabel(tr("Enter a name for the new instance. Its folder holds the mods, downloads, "
                                "profiles and overwrite (MO2's portable layout)."),
                             page);
    intro->setWordWrap(true);
    l->addWidget(intro);
    l->addWidget(new QLabel(tr("Instance name"), page));
    m_name = new QLineEdit(page);
    m_name->setObjectName("instanceName");
    l->addWidget(m_name);
    l->addWidget(new QLabel(tr("Location"), page));
    m_location = new QLineEdit(page);
    m_location->setObjectName("location");
    l->addLayout(pathRow(page, m_location, [this] {
      const QString dir = QFileDialog::getExistingDirectory(this, tr("Instance location"), m_root);
      if (!dir.isEmpty()) {
        m_location->setText(QDir(dir).filePath(m_name->text().trimmed()));
        m_locationEdited = true;
      }
    }));
    m_nameError = errorLabel(page);
    l->addWidget(m_nameError);
    l->addStretch(1);
    connect(m_name, &QLineEdit::textChanged, this, [this](const QString& name) {
      if (!m_locationEdited) {
        QSignalBlocker block(m_location);
        m_location->setText(QDir(m_root).filePath(name));
      }
      validate();
    });
    connect(m_location, &QLineEdit::textEdited, this, [this] {
      m_locationEdited = true;
      validate();
    });
    return page;
  }

  QWidget* protonPage()
  {
    auto* page = new QWidget(this);
    auto* l    = new QVBoxLayout(page);
    auto* intro = new QLabel(tr("The game runs through Proton in a Wine prefix. A new prefix in the instance "
                                "folder keeps instances apart; an existing one can be shared (Strata never "
                                "changes it unless you run its setup)."),
                             page);
    intro->setWordWrap(true);
    l->addWidget(intro);
    l->addWidget(new QLabel(tr("Proton"), page));
    m_proton = new QComboBox(page);
    m_proton->setObjectName("proton");
    int preferred = -1;
    for (std::size_t i = 0; i < m_protons.size(); ++i) {
      m_proton->addItem(m_protons[i].name, m_protons[i].path);
      m_proton->setItemData(int(i), m_protons[i].path, Qt::ToolTipRole);
      if (m_protons[i].name.startsWith("GE-Proton")) {
        preferred = int(i);  // sorted by name: the newest GE-Proton comes last
      }
    }
    if (preferred >= 0) {
      m_proton->setCurrentIndex(preferred);
    }
    l->addWidget(m_proton);
    m_protonNote = errorLabel(page);
    l->addWidget(m_protonNote);
    m_newPrefix = new QRadioButton(tr("A new prefix in the instance folder"), page);
    m_oldPrefix = new QRadioButton(tr("An existing prefix (the folder that holds pfx/)"), page);
    m_newPrefix->setChecked(true);
    l->addWidget(m_newPrefix);
    l->addWidget(m_oldPrefix);
    m_prefix = new QLineEdit(page);
    m_prefix->setObjectName("prefix");
    m_prefix->setEnabled(false);
    l->addLayout(pathRow(page, m_prefix, [this] {
      const QString dir = QFileDialog::getExistingDirectory(this, tr("Prefix"), m_prefix->text());
      if (!dir.isEmpty()) {
        m_oldPrefix->setChecked(true);
        m_prefix->setText(QFileInfo(dir).fileName() == "pfx" ? QFileInfo(dir).path() : dir);
      }
    }));
    m_prefixError = errorLabel(page);
    l->addWidget(m_prefixError);
    m_localInis  = new QCheckBox(tr("Use profile-specific game INI files"), page);
    m_localSaves = new QCheckBox(tr("Use profile-specific save games"), page);
    m_localInis->setChecked(true);
    m_localSaves->setChecked(true);
    l->addWidget(m_localInis);
    l->addWidget(m_localSaves);
    l->addStretch(1);
    connect(m_proton, &QComboBox::currentIndexChanged, this, [this] { validate(); });
    connect(m_oldPrefix, &QRadioButton::toggled, this, [this](bool on) {
      m_prefix->setEnabled(on);
      validate();
    });
    connect(m_prefix, &QLineEdit::textChanged, this, [this] { validate(); });
    return page;
  }

  QWidget* reviewPage()
  {
    auto* page = new QWidget(this);
    auto* l    = new QVBoxLayout(page);
    m_review   = new QPlainTextEdit(page);
    m_review->setObjectName("review");
    m_review->setReadOnly(true);
    l->addWidget(m_review, 1);
    m_setup = new QCheckBox(tr("Set up the prefix now (Proton creates it, then VC++, d3dcompiler, d3dx and .NET "
                               "are installed; downloads, takes a few minutes)"),
                            page);
    m_setup->setObjectName("setupPrefix");
    m_switch = new QCheckBox(tr("Switch to the new instance"), page);
    m_switch->setObjectName("launch");
    m_switch->setChecked(true);
    l->addWidget(m_setup);
    l->addWidget(m_switch);
    m_log = new QPlainTextEdit(page);
    m_log->setObjectName("creationLog");
    m_log->setReadOnly(true);
    m_log->setLineWrapMode(QPlainTextEdit::NoWrap);
    m_log->hide();
    l->addWidget(m_log, 2);
    return page;
  }

  QString gameName() const
  {
    const auto* item = m_gameList->currentItem();
    return item ? item->data(Qt::UserRole).toString() : QString();
  }

  QString folder() const { return QDir::cleanPath(m_location->text().trimmed()); }

  QString prefixText() const
  {
    return m_newPrefix->isChecked() ? QDir(folder()).filePath("Prefix") : QDir::cleanPath(m_prefix->text().trimmed());
  }

  // Shows what is wrong with the current page; returns whether Next may go on.
  bool validate()
  {
    switch (m_pages->currentIndex()) {
    case 0: {
      QString error;
      const QString name = gameName();
      const auto game    = std::find_if(m_games.begin(), m_games.end(), [&](const Game& g) { return g.name == name; });
      if (name.isEmpty()) {
        error = tr("Select a game.");
      } else if (m_gameDir->text().trimmed().isEmpty()) {
        error = tr("Give the game folder: Steam doesn't have this game.");
      } else if (game != m_games.end() && !QFileInfo(QDir(m_gameDir->text().trimmed()).filePath(game->binary)).isFile()) {
        error = tr("This folder has no %1.").arg(game->binary);
      }
      m_gameError->setText(error);
      return error.isEmpty();
    }
    case 1: {
      QString error = qs(strata::instances::name_problem(rs(m_name->text())));
      if (error.isEmpty()) {
        const QDir dir(folder());
        if (folder().isEmpty()) {
          error = tr("Give the location.");
        } else if (dir.exists() && !dir.isEmpty()) {
          error = tr("This folder exists and is not empty.");
        }
      }
      m_nameError->setText(error);
      return error.isEmpty();
    }
    case 2: {
      const int i = m_proton->currentIndex();
      m_protonNote->setText(m_protons.empty() ? tr("No Proton found. Install one (GE-Proton from ProtonUp-Qt, "
                                                   "or Proton from Steam) first.")
                            : i >= 0 ? m_protons[std::size_t(i)].note
                                     : QString());
      QString error;
      if (m_oldPrefix->isChecked()) {
        const QString p = m_prefix->text().trimmed();
        if (p.isEmpty()) {
          error = tr("Give the prefix folder.");
        } else if (!QFileInfo(QDir(p).filePath("pfx")).isDir()) {
          error = tr("This folder holds no pfx/ (give the folder above it).");
        }
      }
      m_prefixError->setText(error);
      return i >= 0 && error.isEmpty();
    }
    default:
      return true;
    }
  }

  void go(int page)
  {
    if (page < 0 || page >= m_pages->count()) {
      return;
    }
    m_pages->setCurrentIndex(page);
    static const char* titles[] = {QT_TR_NOOP("Select a game"), QT_TR_NOOP("Name and location"),
                                   QT_TR_NOOP("Proton and prefix"), QT_TR_NOOP("Review")};
    m_title->setText(tr(titles[page]));
    m_back->setEnabled(page > 0);
    m_next->setText(page == m_pages->count() - 1 ? tr("Create") : tr("Next >"));
    if (page == 1 && m_name->text().isEmpty()) {
      m_name->setText(gameName());  // MO2's default name is the game's
    }
    if (page == 3) {
      m_setup->setChecked(m_newPrefix->isChecked());
      m_review->setPlainText(
          tr("Game: %1\nGame folder: %2\n\nInstance: %3\nFolder: %4\n\nProton: %5\nPrefix: %6%7\n\n"
             "Profile-specific INI files: %8\nProfile-specific saves: %9")
              .arg(gameName(), QDir::cleanPath(m_gameDir->text().trimmed()), m_name->text(), folder(),
                   m_proton->currentData().toString(), prefixText(),
                   m_newPrefix->isChecked() ? tr(" (new)") : tr(" (existing, shared)"),
                   m_localInis->isChecked() ? tr("yes") : tr("no"), m_localSaves->isChecked() ? tr("yes") : tr("no")));
    }
    validate();
  }

  void next()
  {
    if (!m_created.isEmpty()) {
      accept();  // after creating, the button reads "Close" or "Switch"
      return;
    }
    if (!validate()) {
      return;
    }
    if (m_pages->currentIndex() < m_pages->count() - 1) {
      go(m_pages->currentIndex() + 1);
      return;
    }
    create();
  }

  void create()
  {
    strata::instances::NewInstanceSpec spec{
        rs(folder()),
        rs(gameName()),
        rs(QDir::cleanPath(m_gameDir->text().trimmed())),
        rs(m_proton->currentData().toString()),
        rs(m_newPrefix->isChecked() ? QString() : prefixText()),
        m_localInis->isChecked(),
        m_localSaves->isChecked(),
    };
    try {
      m_created = qs(strata::instances::create(std::move(spec)));
    } catch (const std::exception& e) {
      QMessageBox::critical(this, tr("Create new instance"), what(e));
      return;
    }
    qInfo("created the instance %s", qUtf8Printable(m_created));
    m_back->setEnabled(false);
    m_setup->setEnabled(false);
    m_log->show();
    m_log->appendPlainText(tr("Instance created: %1").arg(m_created));
    if (!m_setup->isChecked()) {
      finished();
      return;
    }
    runSetup();
  }

  void runSetup()
  {
    m_next->setEnabled(false);
    m_cancel->setText(tr("Stop"));
    m_process = new QProcess(this);
    m_process->setProcessChannelMode(QProcess::MergedChannels);
    connect(m_process, &QProcess::readyRead, this, [this] {
      m_log->appendPlainText(QString::fromUtf8(m_process->readAll()).trimmed());
    });
    connect(m_process, &QProcess::finished, this, [this](int code, QProcess::ExitStatus status) {
      m_log->appendPlainText(QString::fromUtf8(m_process->readAll()).trimmed());
      m_log->appendPlainText(status == QProcess::NormalExit && code == 0
                                 ? tr("The prefix is set up.")
                                 : tr("The prefix setup did not finish (exit %1). Run it again later: "
                                      "strata prefix setup -i \"%2\" (each step checks first).")
                                       .arg(code)
                                       .arg(m_created));
      m_process->deleteLater();
      m_process = nullptr;
      finished();
    });
    m_log->appendPlainText(tr("Setting up the prefix (strata prefix setup)..."));
    m_process->start(ownProgram(), {"prefix", "setup", "-i", m_created});
  }

  void finished()
  {
    m_next->setEnabled(true);
    m_next->setText(m_switch->isChecked() ? tr("Switch") : tr("Close"));
    m_cancel->setText(tr("Close"));
  }

  void cancel()
  {
    if (m_process) {
      m_process->kill();  // finished() comes from the process' end
      return;
    }
    if (!m_created.isEmpty()) {
      m_switch->setChecked(false);
      accept();
      return;
    }
    reject();
  }

  void reject() override
  {
    if (m_process) {
      return;  // Esc while the setup runs: Stop first
    }
    QDialog::reject();
  }
};

// ---------------------------------------------------------------------------------------
// MO2's InstanceManagerDialog on its own form.
class InstanceManagerDialog : public QDialog
{
public:
  InstanceManagerDialog(QWidget* parent, const QString& current)
      : QDialog(parent), ui(std::make_unique<Ui::InstanceManagerDialog>()), m_current(current)
  {
    ui->setupUi(this);
    ui->splitter->setSizes({250, 1});
    ui->splitter->setStretchFactor(0, 0);
    ui->splitter->setStretchFactor(1, 1);
    // Strata's instances are portable folders: nothing to convert
    ui->convertToGlobal->hide();
    ui->convertToPortable->hide();

    // the prefix and Proton (Strata's [fluorine] keys) below MO2's rows
    auto* grid      = ui->gridLayout;
    m_prefix        = new QLineEdit(this);
    m_proton        = new QLineEdit(this);
    auto* explorePx = new QPushButton(tr("Explore"), this);
    m_prefix->setReadOnly(true);
    m_proton->setReadOnly(true);
    m_prefix->setObjectName("prefix");
    m_proton->setObjectName("proton");
    grid->addWidget(new QLabel(tr("Prefix"), this), 8, 0);
    grid->addWidget(m_prefix, 8, 1);
    grid->addWidget(explorePx, 8, 2);
    grid->addWidget(new QLabel(tr("Proton"), this), 9, 0);
    grid->addWidget(m_proton, 9, 1);
    connect(explorePx, &QPushButton::clicked, this, [this] { explore(m_prefix->text()); });

    // Leet change 1: the instances root, in the bottom row
    m_rootButton = new QPushButton(this);
    m_rootButton->setObjectName("instancesRoot");
    m_rootButton->setToolTip(tr("The folder whose instances are listed (Leet Edition's instances root). "
                                "Instances opened elsewhere are listed too."));
    ui->horizontalLayout_3->insertWidget(0, new QLabel(tr("Instances folder:"), this));
    ui->horizontalLayout_3->insertWidget(1, m_rootButton);
    showRoot();
    connect(m_rootButton, &QPushButton::clicked, this, [this] { chooseRoot(); });

    m_proxy.setSourceModel(&m_model);
    m_proxy.setFilterCaseSensitivity(Qt::CaseInsensitive);
    ui->list->setModel(&m_proxy);
    connect(ui->filter, &QLineEdit::textChanged, &m_proxy, &QSortFilterProxyModel::setFilterFixedString);

    reload();
    select(m_current);

    connect(ui->list->selectionModel(), &QItemSelectionModel::selectionChanged, this, [this] { fill(); });
    connect(ui->list, &QListView::activated, this, [this] { switchTo(); });
    connect(ui->createNew, &QPushButton::clicked, this, [this] { createNew(); });
    connect(ui->rename, &QPushButton::clicked, this, [this] { rename(); });
    connect(ui->exploreLocation, &QPushButton::clicked, this, [this] { explore(ui->location->text()); });
    connect(ui->exploreBaseDirectory, &QPushButton::clicked, this, [this] { explore(ui->baseDirectory->text()); });
    connect(ui->exploreGame, &QPushButton::clicked, this, [this] { explore(ui->gameDir->text()); });
    connect(ui->openINI, &QPushButton::clicked, this, [this] {
      if (const Row* r = selected()) {
        QDesktopServices::openUrl(QUrl::fromLocalFile(QDir(r->root).filePath("ModOrganizer.ini")));
      }
    });
    connect(ui->deleteInstance, &QPushButton::clicked, this, [this] { deleteInstance(); });
    connect(ui->switchToInstance, &QPushButton::clicked, this, [this] { switchTo(); });
    connect(ui->close, &QPushButton::clicked, this, &QDialog::reject);
  }

  QString chosen() const { return m_chosen; }

private:
  std::unique_ptr<Ui::InstanceManagerDialog> ui;
  QString m_current;
  QString m_chosen;
  std::vector<Row> m_rows;
  QStandardItemModel m_model;
  QSortFilterProxyModel m_proxy;
  QLineEdit* m_prefix        = nullptr;
  QLineEdit* m_proton        = nullptr;
  QPushButton* m_rootButton  = nullptr;

  void showRoot() { m_rootButton->setText(qs(strata::instances::instances_root())); }

  void chooseRoot()
  {
    const QString dir = QFileDialog::getExistingDirectory(this, tr("Instances folder"), m_rootButton->text());
    if (dir.isEmpty()) {
      return;
    }
    try {
      strata::instances::set_instances_root(rs(dir));
    } catch (const std::exception& e) {
      QMessageBox::critical(this, tr("Instances folder"), what(e));
      return;
    }
    showRoot();
    const QString keep = selected() ? selected()->root : m_current;
    reload();
    select(keep);
  }

  void reload()
  {
    m_rows.clear();
    m_model.clear();
    for (const auto& r : strata::instances::list(rs(m_current))) {
      Row row{qs(r.name), qs(r.root), qs(r.base_dir), qs(r.game_name), qs(r.game_dir), qs(r.prefix), qs(r.proton),
              r.supported};
      auto* item = new QStandardItem(row.name);
      item->setEditable(false);
      item->setToolTip(row.root);
      if (row.root == m_current) {
        QFont f = item->font();
        f.setBold(true);
        item->setFont(f);
        item->setToolTip(tr("%1 (open now)").arg(row.root));
      }
      if (!row.supported) {
        QFont f = item->font();
        f.setItalic(true);
        item->setFont(f);
        item->setToolTip(tr("%1\nStrata doesn't support the game \"%2\"").arg(row.root, row.game));
      }
      m_model.appendRow(item);
      m_rows.push_back(std::move(row));
    }
    fill();
  }

  void select(const QString& root)
  {
    int found = m_rows.empty() ? -1 : 0;
    for (std::size_t i = 0; i < m_rows.size(); ++i) {
      if (m_rows[i].root == root) {
        found = int(i);
      }
    }
    if (found >= 0) {
      const QModelIndex index = m_proxy.mapFromSource(m_model.index(found, 0));
      ui->list->selectionModel()->select(index, QItemSelectionModel::ClearAndSelect);
      ui->list->setCurrentIndex(index);
      ui->list->scrollTo(index);
    }
    fill();
  }

  const Row* selected() const
  {
    const auto rows = ui->list->selectionModel() ? ui->list->selectionModel()->selectedRows() : QModelIndexList();
    if (rows.size() != 1) {
      return nullptr;
    }
    const int r = m_proxy.mapToSource(rows.first()).row();
    return r >= 0 && r < int(m_rows.size()) ? &m_rows[std::size_t(r)] : nullptr;
  }

  void fill()
  {
    const Row* r = selected();
    ui->name->setText(r ? r->name : QString());
    ui->location->setText(r ? r->root : QString());
    ui->baseDirectory->setText(r ? r->base : QString());
    ui->gameName->setText(r ? r->game : QString());
    ui->gameDir->setText(r ? r->gameDir : QString());
    m_prefix->setText(r ? r->prefix : QString());
    m_proton->setText(r ? r->proton : QString());
    for (QLineEdit* e : {ui->name, ui->location, ui->baseDirectory, ui->gameName, ui->gameDir, m_prefix, m_proton}) {
      e->setCursorPosition(0);  // the start of a long path, not its end
    }
    for (QPushButton* b : {ui->rename, ui->exploreLocation, ui->exploreBaseDirectory, ui->exploreGame, ui->openINI,
                           ui->deleteInstance, ui->switchToInstance}) {
      b->setEnabled(r != nullptr);
    }
    if (r) {
      ui->switchToInstance->setEnabled(r->supported && r->root != m_current);
    }
  }

  void createNew()
  {
    CreateInstanceDialog dlg(this);
    dlg.exec();
    if (dlg.created().isEmpty()) {
      return;
    }
    if (dlg.switching()) {
      m_chosen = dlg.created();
      accept();
      return;
    }
    reload();
    select(dlg.created());
  }

  void rename()
  {
    const Row* r = selected();
    if (!r) {
      return;
    }
    if (r->root == m_current) {
      QMessageBox::information(this, tr("Rename instance"), tr("The active instance cannot be renamed."));
      return;
    }
    bool ok            = false;
    const QString name = QInputDialog::getText(
        this, tr("Rename instance"),
        tr("Instance name\n\nThe folder is renamed and the paths into it in ModOrganizer.ini are rewritten. "
           "Other programs that point into it (Fluorine's settings, Steam shortcuts) are not."),
        QLineEdit::Normal, r->name, &ok);
    if (!ok || name == r->name) {
      return;
    }
    try {
      const QString target = qs(strata::instances::rename(rs(r->root), rs(name)));
      reload();
      select(target);
    } catch (const std::exception& e) {
      QMessageBox::critical(this, tr("Error"), tr("Failed to rename \"%1\" to \"%2\": %3").arg(r->name, name, what(e)));
    }
  }

  void deleteInstance()
  {
    const Row* r = selected();
    if (!r) {
      return;
    }
    if (r->root == m_current) {
      QMessageBox::information(this, tr("Deleting instance"), tr("The active instance cannot be deleted."));
      return;
    }
    if (const QString busy = qs(strata::instances::busy(rs(r->root))); !busy.isEmpty()) {
      QMessageBox::information(this, tr("Deleting instance"), busy);
      return;
    }
    // MO2's TaskDialog: the list of what goes, the instance folder ticked and fixed
    QDialog dlg(this);
    dlg.setObjectName("deleteInstanceDialog");
    dlg.setWindowTitle(tr("Deleting instance"));
    dlg.resize(600, 300);
    auto* l     = new QVBoxLayout(&dlg);
    auto* main  = new QLabel(tr("<b>These files and folders will be deleted</b>"), &dlg);
    auto* note  = new QLabel(tr("All checked items will be deleted. A prefix or game folder outside the instance "
                                "folder is never deleted here."),
                             &dlg);
    note->setWordWrap(true);
    auto* list = new QListWidget(&dlg);
    for (const auto& c : strata::instances::deletion_candidates(rs(r->root))) {
      auto* item = new QListWidgetItem(qs(c.path), list);
      if (c.mandatory) {
        item->setFlags(item->flags() & ~Qt::ItemIsEnabled);
        item->setCheckState(Qt::Checked);
      } else {
        item->setFlags(item->flags() | Qt::ItemIsUserCheckable);
        item->setCheckState(Qt::Unchecked);
      }
    }
    l->addWidget(main);
    l->addWidget(note);
    l->addWidget(list, 1);
    auto* box     = new QDialogButtonBox(&dlg);
    auto* recycle = box->addButton(tr("Move to the trash"), QDialogButtonBox::AcceptRole);
    auto* permanent = box->addButton(tr("Delete permanently"), QDialogButtonBox::DestructiveRole);
    box->addButton(QDialogButtonBox::Cancel);
    QAbstractButton* clicked = nullptr;
    connect(box, &QDialogButtonBox::clicked, &dlg, [&](QAbstractButton* b) {
      clicked = b;
      dlg.done(b == recycle || b == permanent ? QDialog::Accepted : QDialog::Rejected);
    });
    l->addWidget(box);
    if (dlg.exec() != QDialog::Accepted) {
      return;
    }
    rust::Vec<rust::String> paths;
    for (int i = 0; i < list->count(); ++i) {
      if (list->item(i)->checkState() == Qt::Checked) {
        paths.push_back(rs(list->item(i)->text()));
      }
    }
    if (paths.empty()) {
      QMessageBox::information(this, tr("Deleting instance"), tr("Nothing to delete."));
      return;
    }
    try {
      strata::instances::delete_paths(rs(r->root), std::move(paths), clicked == recycle);
    } catch (const std::exception& e) {
      QMessageBox::critical(this, tr("Deleting instance"), what(e));
    }
    reload();
  }

  void switchTo()
  {
    const Row* r = selected();
    if (!r || r->root == m_current) {
      return;
    }
    if (!r->supported) {
      QMessageBox::information(this, tr("Switching instances"),
                               tr("Strata doesn't support the game \"%1\" yet.").arg(r->game));
      return;
    }
    m_chosen = r->root;
    accept();
  }
};

}  // namespace

QString showInstanceManager(QWidget* parent, const QString& current)
{
  InstanceManagerDialog dlg(parent, current);
  if (!qEnvironmentVariableIsEmpty("STRATA_SHELL_DIALOG_SCREENSHOT")) {
    dlg.show();
    dlg.grab().save(qEnvironmentVariable("STRATA_SHELL_DIALOG_SCREENSHOT"));
    return {};
  }
  dlg.exec();
  return dlg.chosen();
}

QString showCreateInstance(QWidget* parent, QString* created)
{
  CreateInstanceDialog dlg(parent);
  dlg.exec();
  if (created) {
    *created = dlg.created();
  }
  return dlg.switching() ? dlg.created() : QString();
}

}  // namespace strata::instances_ui

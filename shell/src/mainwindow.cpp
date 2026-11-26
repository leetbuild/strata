#include "mainwindow.h"

#include "columnfit.h"
#include "conflicts/modinfodialog.h"
#include "strata_bridge/bridge_install.h"
#include "widgets/columnmenu.h"
#include "conflicts/preview.h"

#include "datatab.h"
#include "filterpanel.h"
#include "install/downloadactions.h"
#include "install/installhost.h"
#include "install/downloadqueue.h"
#include "strata_bridge/bridge_builtins.h"
#include "strata_bridge/bridge_crashlogs.h"
#include "pluginstab.h"
#include "geometrysaver.h"
#include "overwritedialog.h"
#include "layoutstate.h"
#include "modlist/deleteconfirm.h"
#include "strata_bridge/bridge_modlist.h"
#include "modlist/dialogs.h"
#include "instances/instancedialogs.h"
#include "listtools.h"
#include "style.h"
#include "ui_mainwindow.h"

#include <QCloseEvent>
#include <QDateTime>
#include <QDesktopServices>
#include <QDir>
#include <QFileDialog>
#include <QApplication>
#include <QElapsedTimer>
#include <QLocalServer>
#include <QLocalSocket>
#include <QPointer>
#include <QFileInfo>
#include <QLabel>
#include <QLocale>
#include <QMenu>
#include <QProcess>
#include <QTextDocumentFragment>
#include <QScrollBar>
#include <QColorDialog>
#include <QInputDialog>
#include <QMessageBox>
#include <QItemSelection>
#include <QDialogButtonBox>
#include <QFormLayout>
#include <QRegularExpression>
#include <QSpinBox>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTimer>
#include <QUrl>
#include <QToolButton>
#include <QWhatsThis>

#include <climits>
#include <thread>
#include <utility>

namespace
{

QString qs(const rust::String& s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

std::string utf8(const QString& s)
{
  return s.toStdString();
}

rust::Vec<rust::String> rustNames(const QStringList& names)
{
  rust::Vec<rust::String> out;
  for (const QString& n : names) {
    out.push_back(rust::String(n.toStdString()));
  }
  return out;
}

constexpr int SortRole = Qt::UserRole + 10;  // numbers sort as numbers

// The toolbar sizes of MO2's View > Toolbars menu (mainwindow.cpp).
const QSize SmallToolbarSize(24, 24);
const QSize MediumToolbarSize(32, 32);
const QSize LargeToolbarSize(42, 36);

// A read-only item for the simple lists.
QStandardItem* cell(const QString& text, const QVariant& sortKey = {})
{
  auto* item = new QStandardItem(text);
  item->setEditable(false);
  item->setData(sortKey.isValid() ? sortKey : QVariant(text.toLower()), SortRole);
  return item;
}

QString dateText(qint64 secs)
{
  return secs < 0 ? QString()
                  : QLocale().toString(QDateTime::fromSecsSinceEpoch(secs), QLocale::ShortFormat);
}

// Every action that has no behaviour yet: visible as in MO2, greyed, saying why.
void notYet(QAction* action, const QString& why)
{
  action->setEnabled(false);
  action->setToolTip(action->text().remove('&') + " (" + why + ")");
}

}  // namespace

MainWindow::MainWindow(Style& style, QWidget* parent)
    : QMainWindow(parent), ui(std::make_unique<Ui::MainWindow>()), m_style(style)
{
  ui->setupUi(this);
  setWindowTitle(QStringLiteral("Strata v%1").arg(qs(strata::version())));

  m_statusText = new QLabel(this);
  m_statusText->setObjectName("statusText");
  ui->statusBar->addWidget(m_statusText, 1);
  // A temporary message (an action's status tip under the mouse) takes the bar's left part.
  // QStatusBar hides the normal widgets for it, but only those visible at that moment: a tip
  // shown before the window was, left both texts drawn on top of each other. The label
  // follows the message itself.
  connect(ui->statusBar, &QStatusBar::messageChanged, m_statusText,
          [this](const QString& message) { m_statusText->setVisible(message.isEmpty()); });

  setupLists();
  setupMenus();

  m_layout = new LayoutState(this);
  m_layout->addSplitter(ui->splitter);
  m_layout->addSplitter(ui->categoriesSplitter);
  m_layout->addVisibility(ui->menuBar);
  m_layout->addVisibility(ui->statusBar);
  m_layout->addVisibility(ui->categoriesGroup);
  m_layout->addIndex(ui->tabWidget);
  m_layout->addIndex(ui->executablesListBox);
  // Fluorine's lists have more columns: the mod list adds Author and Uploader after Category
  // (13), the plugin list Form Version, Header Version, Author and Description at the end (8)
  // (Fluorine-Manager src/src/modlist.h, pluginlist.h); their saved states are read column
  // by column
  // MO2's header menus (setCustomizableColumns, DownloadListView's own): show / hide columns
  for (QTreeView* view : {static_cast<QTreeView*>(ui->modList), static_cast<QTreeView*>(ui->espList),
                          static_cast<QTreeView*>(ui->downloadView), ui->dataTree, ui->rootTree}) {
    strata::setCustomizableColumns(view);
  }
  m_layout->addHeader(ui->modList->header(), {{0, 1, 2, 3, 4, -1, -1, 5, 6, 7, 8, 9, 10}});
  m_layout->addHeader(ui->espList->header(), {{0, 1, 2, 3, -1, -1, -1, -1}});
  m_layout->addHeader(ui->downloadView->header());
  m_layout->addHeader(ui->savegameList->header());
  m_layout->addHeader(ui->dataTree->header());
  m_layout->addHeader(ui->rootTree->header());
  m_layout->addList("MainWindow_modList_index", [this]() -> std::optional<QStringList> {
    if (!m_modNested || !m_expandedLoaded) {
      return std::nullopt;
    }
    // in list order, as WidgetSettings::saveTreeExpandState walks the view
    QStringList open;
    std::vector<QStandardItem*> stack;
    for (int r = m_modModel->rowCount() - 1; r >= 0; --r) {
      stack.push_back(m_modModel->item(r));
    }
    while (!stack.empty()) {
      auto* item = stack.back();
      stack.pop_back();
      if (item->hasChildren()) {
        if (m_expanded.contains(item->text())) {
          open << item->text();
        }
        for (int c = item->rowCount() - 1; c >= 0; --c) {
          stack.push_back(item->child(c));
        }
      }
    }
    return open;
  });
  // a separator opened or closed by the user (not by the window restoring the state)
  for (auto signal : {&QTreeView::expanded, &QTreeView::collapsed}) {
    connect(ui->modList, signal, m_layout, [this] {
      if (!m_applyingExpand) {
        m_layout->touch();
      }
    });
  }
}

MainWindow::~MainWindow() = default;

// The lists, tabs and rows of the window, one step each.
void MainWindow::setupLists()
{
  setupModList();
  setupModListActions();
  setupPluginsAndDownloads();
  setupDataTabs();
  setupProfileRow();
  setupRun();
  listenForLinks();
}

void MainWindow::listenForLinks()
{
  QString path;
  try {
    path = qs(strata::handover_socket_path());
  } catch (const std::exception& e) {
    qWarning("nxm links from the browser: %s", e.what());
    return;
  }
  m_linkServer = new QLocalServer(this);
  m_linkServer->setSocketOptions(QLocalServer::UserAccessOption);
  QLocalServer::removeServer(path);  // a window of this pid that died (the pid came back)
  if (!m_linkServer->listen(path)) {
    qWarning("nxm links from the browser: cannot listen on %s: %s", qUtf8Printable(path),
             qUtf8Printable(m_linkServer->errorString()));
    return;
  }
  // one line in (the link), one line out: "ok" or "no <why>"
  connect(m_linkServer, &QLocalServer::newConnection, this, [this] {
    while (QLocalSocket* socket = m_linkServer->nextPendingConnection()) {
      connect(socket, &QLocalSocket::disconnected, socket, &QObject::deleteLater);
      connect(socket, &QLocalSocket::readyRead, socket, [this, socket] {
        if (!socket->canReadLine()) {
          return;
        }
        const QString link = QString::fromUtf8(socket->readLine()).trimmed();
        QString why = m_session ? QString() : tr("no instance is open");
        if (why.isEmpty()) {
          const rust::String root = (*m_session)->root();
          const QByteArray utf8   = link.toUtf8();
          why = qs(strata::handover_refusal(root, rust::Str(utf8.data(), std::size_t(utf8.size()))));
        }
        if (why.isEmpty() && !m_downloadQueue) {
          why = tr("the download queue is not ready");
        }
        socket->write(why.isEmpty() ? QByteArray("ok\n") : ("no " + why + "\n").toUtf8());
        socket->flush();
        socket->disconnectFromServer();
        if (why.isEmpty()) {
          qInfo("nxm link from the browser: %s", qUtf8Printable(link));
          addDownloadLink(link);
        }
      });
    }
  });
}

// The mod list's models, proxy, header, filter box and open separators (ModListView::setup).
void MainWindow::setupModList()
{
  for (auto& model : m_modModels) {
    model.setColumnCount(modlist::ColumnCount);
    model.setHorizontalHeaderLabels(modlist::columnNames());
  }
  m_modProxy.setSourceModel(m_modModel);
  ui->modList->setModel(&m_modProxy);
  ui->modList->setSortingEnabled(true);
  ui->modList->sortByColumn(modlist::ColPriority, Qt::AscendingOrder);
  auto* header = ui->modList->header();
  // hidden by default; a saved header state overrides this
  for (int c : {modlist::ColContent, modlist::ColModId, modlist::ColGame,
                modlist::ColInstallTime, modlist::ColNotes}) {
    header->setSectionHidden(c, true);
  }
  for (int i = 0; i < header->count(); ++i) {
    header->setSectionResizeMode(i, QHeaderView::Interactive);
  }
  header->setStretchLastSection(true);  // MainWindow::allowListResize
  header->resizeSection(modlist::ColName, 320);
  connect(header, &QHeaderView::sortIndicatorChanged, this, &MainWindow::onModSortChanged);
  connect(header, &QHeaderView::sectionResized, this, [this](int logical, int, int newSize) {
    m_modProxy.setColumnVisible(logical, newSize != 0);
  });
  connect(ui->modFilterEdit, &QLineEdit::textChanged, this, [this](const QString& text) {
    m_modProxy.setFilterText(text);
    applyExpandState();  // ModListView: refreshExpandedItems after the filter changed
  });
  connect(ui->modList, &QTreeView::expanded, this, [this](const QModelIndex& index) {
    if (!m_applyingExpand) {
      m_expanded.insert(index.data(Qt::DisplayRole).toString());
    }
  });
  // a separator opened or closed changes what the selection's highlight covers (conflicts_from)
  connect(ui->modList, &QTreeView::expanded, this, &MainWindow::updateMarkers);
  connect(ui->modList, &QTreeView::collapsed, this, &MainWindow::updateMarkers);
  connect(ui->modList, &QTreeView::collapsed, this, [this](const QModelIndex& index) {
    if (!m_applyingExpand) {
      m_expanded.remove(index.data(Qt::DisplayRole).toString());
    }
  });
  for (auto signal : {&QAbstractItemModel::rowsInserted, &QAbstractItemModel::rowsRemoved}) {
    connect(&m_modProxy, signal, this, &MainWindow::updateModCounter);
  }
  connect(&m_modProxy, &QAbstractItemModel::modelReset, this, &MainWindow::updateModCounter);
  for (auto& model : m_modModels) {
    connect(&model, &QStandardItemModel::itemChanged, this, &MainWindow::onModItemChanged);
  }
}

// What the user does on the mod list: double click, drops, keys, selection, context menu.
void MainWindow::setupModListActions()
{
  // conflicts area: double click on a mod opens its information (MO2's default action).
  connect(ui->modList, &QTreeView::doubleClicked, this, [this](const QModelIndex& index) {
    const QString mod = index.siblingAtColumn(0).data(modlist::InternalNameRole).toString();
    if (index.siblingAtColumn(0).data(modlist::KindRole).toInt() == modlist::Overwrite) {
      openOverwrite();  // MO2: ModListViewActions::displayModInformation → OverwriteInfoDialog
    } else if (!mod.isEmpty()) {
      openModInfo(mod);
    }
  });
  connect(ui->modList, &ModListView::dropRequested, this, &MainWindow::onModDrop);
  connect(ui->modList, &ModListView::foreignDropRequested, this, &MainWindow::onForeignDrop);
  // ModListView::event's keys
  connect(ui->modList, &ModListView::levelRequested, this, [this](const QModelIndex& index) {
    editSeparatorLevel(index.data(modlist::InternalNameRole).toString());
  });
  connect(ui->modList, &ModListView::openFolderRequested, this, [this](const QModelIndex& index) {
    const QByteArray n = index.data(modlist::InternalNameRole).toString().toUtf8();
    if (m_session && index.data(modlist::KindRole).toInt() != modlist::Foreign) {
      openFolder(qs((*m_session)->mod_path(rust::Str(n.data(), std::size_t(n.size())))));
    }
  });
  connect(ui->modList, &ModListView::moveRequested, this, [this](bool up) {
    // ModListView::moveSelection: only while sorted by priority; Up is towards the top of
    // the screen, whichever way the priorities run
    const auto* header = ui->modList->header();
    if (header->sortIndicatorSection() != modlist::ColPriority) {
      return;
    }
    qint64 offset = up ? -1 : 1;
    if (header->sortIndicatorOrder() == Qt::DescendingOrder) {
      offset = -offset;
    }
    const QStringList names = selectedMods();
    modOperation("move", [&](strata::Session& s) { return s.shift_mods(rustNames(names), offset); });
  });
  connect(ui->modList, &ModListView::removeRequested, this, [this] { removeMods(selectedMods()); });
  connect(ui->modList, &ModListView::toggleRequested, this, [this] {
    const QStringList names = selectedMods();
    modOperation("toggle", [&](strata::Session& s) { return s.toggle_mods(rustNames(names)); });
  });
  connect(ui->modList->selectionModel(), &QItemSelectionModel::selectionChanged, this,
          &MainWindow::updateMarkers);
  // ModListView::setup: the icon columns' delegates and their compact widths
  ui->modList->setItemDelegateForColumn(modlist::ColFlags, new IconDelegate(ui->modList, modlist::ColFlags, 120, &m_rollUps));
  ui->modList->setItemDelegateForColumn(modlist::ColConflictFlags,
                                        new IconDelegate(ui->modList, modlist::ColConflictFlags, 80, &m_rollUps));
  ui->modList->setItemDelegateForColumn(modlist::ColContent, new IconDelegate(ui->modList, modlist::ColContent, 150, &m_rollUps));
  ui->modList->setSelectionMode(QAbstractItemView::ExtendedSelection);
  ui->modList->setContextMenuPolicy(Qt::CustomContextMenu);
  connect(ui->modList, &QWidget::customContextMenuRequested, this, &MainWindow::showModContextMenu);
  connect(&m_modProxy, &QAbstractItemModel::layoutChanged, this, &MainWindow::updateModCounter);
}

// The Plugins and Saves tabs (PluginsTab), the Downloads tab (DownloadListView).
void MainWindow::setupPluginsAndDownloads()
{
  m_pluginsTab = new PluginsTab(ui.get(), this);
  connect(m_pluginsTab, &PluginsTab::error, this, &MainWindow::showError);
  connect(m_pluginsTab, &PluginsTab::filled, this, [this] {
    if (m_fitPlugins) {
      m_fitPlugins = false;  // after the first fill (it runs on a worker): the rows are there
      if (!m_pluginFit) {
        m_pluginFit = new ColumnFit(ui->espList, 0, m_layout);
      }
      m_pluginFit->start();
    }
  });

  // ---- downloads (DownloadListView)
  m_downloadModel.setHorizontalHeaderLabels({tr("Name"), tr("Status"), tr("Size"),
                                             tr("Filetime"), tr("Mod name"), tr("Version"),
                                             tr("Nexus ID"), tr("Source Game")});
  m_downloadProxy.setSourceModel(&m_downloadModel);
  m_downloadProxy.setSortRole(SortRole);
  m_downloadProxy.setFilterCaseSensitivity(Qt::CaseInsensitive);
  m_downloadProxy.setFilterKeyColumn(0);
  ui->downloadView->setModel(&m_downloadProxy);
  ui->downloadView->setSortingEnabled(true);
  ui->downloadView->sortByColumn(1, Qt::DescendingOrder);
  ui->downloadView->header()->setStretchLastSection(false);
  ui->downloadView->header()->resizeSection(0, 400);
  for (int c = 4; c < 8; ++c) {
    ui->downloadView->header()->hideSection(c);  // DownloadListView::setManager
  }
  connect(ui->downloadFilterEdit, &QLineEdit::textChanged, &m_downloadProxy,
          &QSortFilterProxyModel::setFilterFixedString);
  connect(ui->showHiddenBox, &QCheckBox::toggled, this, &MainWindow::loadDownloads);
  connect(ui->btnRefreshDownloads, &QPushButton::clicked, this, &MainWindow::loadDownloads);
  // install area: install on double click, the download context menu, the nxm queue
  m_downloadQueue = strata::attachDownloadActions(ui->downloadView, this, downloadContext());
}

// The Data and Root tabs.
void MainWindow::setupDataTabs()
{
  m_dataTab = new DataTab(ui->dataTree, ui->dataTabRefresh, 0, ui->dataTabFilter,
                          ui->dataTabShowOnlyConflicts, ui->dataTabShowFromArchives, this);
  m_rootTab = new DataTab(ui->rootTree, ui->rootTabRefresh, 1, ui->rootTabFilter,
                          ui->rootTabShowOnlyConflicts, nullptr, this);
  ui->dataTree->header()->resizeSection(0, 300);
  ui->rootTree->header()->resizeSection(0, 300);
  connect(ui->tabWidget, &QTabWidget::currentChanged, this, [this](int index) {
    if (ui->tabWidget->widget(index) == ui->dataTab) {
      m_dataTab->activate();
    } else if (ui->tabWidget->widget(index) == ui->rootTab) {
      m_rootTab->activate();
    }
  });
}

// The row above the mod list: profile combo, list options, backups, the filter panel.
void MainWindow::setupProfileRow()
{
  connect(ui->profileBox, &QComboBox::activated, this, &MainWindow::switchProfile);
  // the list options button: ModListGlobalContextMenu, filled when it opens
  auto* listOptions = new QMenu(this);
  ui->listOptionsBtn->setMenu(listOptions);
  connect(listOptions, &QMenu::aboutToShow, this, [this, listOptions] {
    listOptions->clear();
    if (m_session) {
      fillGlobalModMenu(*listOptions, {});
    }
  });
  // MO2's shortcut button (toolbar / desktop / start menu links to an executable)
  ui->linkButton->setEnabled(false);
  ui->linkButton->setToolTip(tr("Shortcuts to executables are not in Strata yet (no milestone has them)."));
  connect(ui->saveModsButton, &QPushButton::clicked, this, [this] { backupOrRestore(true, false); });
  connect(ui->restoreModsButton, &QPushButton::clicked, this, [this] { backupOrRestore(true, true); });
  connect(ui->saveButton, &QPushButton::clicked, this, [this] { backupOrRestore(false, false); });
  connect(ui->restoreButton, &QPushButton::clicked, this, [this] { backupOrRestore(false, true); });
  m_filterPanel = new FilterPanel(ui->filters, ui->filtersAnd, ui->filtersOr,
                                  ui->filtersSeparators, ui->filtersClear, this);
  connect(m_filterPanel, &FilterPanel::changed, this, &MainWindow::applyCriteria);
  connect(ui->clearFiltersButton, &QPushButton::clicked, ui->filtersClear, &QPushButton::click);
  ui->currentCategoryLabel->clear();
  // the categories panel is hidden unless the ini says otherwise (readSettings)
  ui->categoriesGroup->setVisible(false);
  ui->displayCategoriesBtn->setCheckable(true);
  connect(ui->displayCategoriesBtn, &QPushButton::toggled, ui->categoriesGroup,
          &QWidget::setVisible);
}

// The executables combo and Run.
void MainWindow::setupRun()
{
  // ---- run; "<Edit...>" opens the executables dialog, then the previous entry is selected
  // again (MainWindow::on_executablesListBox_currentIndexChanged)
  connect(ui->executablesListBox, &QComboBox::currentIndexChanged, this, [this](int index) {
    const int previous = m_oldExecutableIndex > 0 ? m_oldExecutableIndex : 1;
    m_oldExecutableIndex = index;
    if (index != 0 || !m_session) {
      return;
    }
    QTimer::singleShot(0, this, [this, previous] {
      openDialog("executables");
      const int count = ui->executablesListBox->count();
      ui->executablesListBox->setCurrentIndex(previous < count ? previous : count - 1);
    });
  });
  connect(ui->startButton, &QPushButton::clicked, this, [this] {
    if (ui->executablesListBox->currentIndex() > 0) {
      runExecutable(ui->executablesListBox->currentText());
    }
  });
}

void MainWindow::setupMenus()
{
  connect(ui->actionExit, &QAction::triggered, this, &QWidget::close);
  connect(ui->action_Refresh, &QAction::triggered, this, &MainWindow::refresh);

  // View > Toolbars (MainWindow::on_action*_triggered)
  connect(ui->actionMainMenuToggle, &QAction::triggered, ui->menuBar, &QWidget::setVisible);
  connect(ui->actionToolBarMainToggle, &QAction::triggered, ui->toolBar, &QWidget::setVisible);
  connect(ui->actionStatusBarToggle, &QAction::triggered, ui->statusBar, &QWidget::setVisible);
  connect(ui->actionViewLog, &QAction::triggered, ui->logDock, &QWidget::setVisible);
  connect(ui->actionToolBarSmallIcons, &QAction::triggered, this,
          [this] { ui->toolBar->setIconSize(SmallToolbarSize); });
  connect(ui->actionToolBarMediumIcons, &QAction::triggered, this,
          [this] { ui->toolBar->setIconSize(MediumToolbarSize); });
  connect(ui->actionToolBarLargeIcons, &QAction::triggered, this,
          [this] { ui->toolBar->setIconSize(LargeToolbarSize); });
  connect(ui->actionToolBarIconsOnly, &QAction::triggered, this,
          [this] { ui->toolBar->setToolButtonStyle(Qt::ToolButtonIconOnly); });
  connect(ui->actionToolBarTextOnly, &QAction::triggered, this,
          [this] { ui->toolBar->setToolButtonStyle(Qt::ToolButtonTextOnly); });
  connect(ui->actionToolBarIconsAndText, &QAction::triggered, this,
          [this] { ui->toolBar->setToolButtonStyle(Qt::ToolButtonTextUnderIcon); });
  // MainWindow::updateViewMenu: tick what is current when the menu opens
  connect(ui->menuView, &QMenu::aboutToShow, this, [this] {
    ui->actionMainMenuToggle->setChecked(ui->menuBar->isVisible());
    ui->actionToolBarMainToggle->setChecked(ui->toolBar->isVisible());
    ui->actionStatusBarToggle->setChecked(ui->statusBar->isVisible());
    ui->actionViewLog->setChecked(ui->logDock->isVisible());
    ui->actionToolBarSmallIcons->setChecked(ui->toolBar->iconSize() == SmallToolbarSize);
    ui->actionToolBarMediumIcons->setChecked(ui->toolBar->iconSize() == MediumToolbarSize);
    ui->actionToolBarLargeIcons->setChecked(ui->toolBar->iconSize() == LargeToolbarSize);
    const auto style = ui->toolBar->toolButtonStyle();
    ui->actionToolBarIconsOnly->setChecked(style == Qt::ToolButtonIconOnly);
    ui->actionToolBarTextOnly->setChecked(style == Qt::ToolButtonTextOnly);
    ui->actionToolBarIconsAndText->setChecked(style == Qt::ToolButtonTextUnderIcon);
  });

  // install area: Install Mod... and Auto-enable installed mods
  m_installActions =
      strata::attachInstallActions(ui->actionInstallMod, ui->actionAutoEnableMods, this, downloadContext()).sync;
  // MainWindow::on_actionNexus_triggered: the game's page in the browser (no account needed)
  connect(ui->actionNexus, &QAction::triggered, this, [this] {
    if (!m_session) {
      return;
    }
    const QString url = qs(strata::nexus_game_url(utf8(qs((*m_session)->game_name()))));
    if (url.isEmpty()) {
      showError(tr("Strata doesn't know the Nexus page of %1").arg(qs((*m_session)->game_name())));
    } else {
      QDesktopServices::openUrl(QUrl(url));
    }
  });
  // updateModPageMenu: with one game and no mod page plugins MO2 hides the mod page menu
  // and shows "Visit Nexus" in its place
  ui->actionModPage->setVisible(false);
  notYet(ui->actionEndorseMO, tr("not in Strata: it endorses Mod Organizer on Nexus"));
  notYet(ui->actionUpdate, tr("not in Strata: update it from git and tools/build.sh"));
  // the Help menu (MainWindow::updateHelpMenu), the parts that apply
  auto* help = new QMenu(this);
  help->addAction(tr("Help on UI"), this, [] { QWhatsThis::enterWhatsThisMode(); });
  notYet(help->addAction(tr("Documentation")), tr("not yet: Strata's documentation is its README for now"));
  help->addSeparator();
  help->addAction(tr("About"), this, [this] {
    QMessageBox::about(this, tr("About Strata"),
                       tr("<b>Strata</b> %1<br>A Linux-native mod manager: Mod Organizer 2.5.2 with the "
                          "Leet Edition's changes.<br>GPL-3.0").arg(qs(strata::version())));
  });
  help->addAction(tr("About Qt"), qApp, &QApplication::aboutQt);
  ui->actionHelp->setMenu(help);
  // setupActionMenu: a click on the toolbar button opens the menu at once
  if (auto* button = qobject_cast<QToolButton*>(ui->toolBar->widgetForAction(ui->actionHelp))) {
    button->setPopupMode(QToolButton::InstantPopup);
  }
  // the modlist area's dialogs
  connect(ui->actionAdd_Profile, &QAction::triggered, this, [this] { openDialog("profiles"); });
  connect(ui->actionModify_Executables, &QAction::triggered, this, [this] { openDialog("executables"); });
  connect(ui->actionSettings, &QAction::triggered, this, [this] { openDialog("settings"); });
  connect(ui->actionChange_Game, &QAction::triggered, this, &MainWindow::showInstanceManager);
  // Tool Plugins (MainWindow::updateToolMenu): the INI Editor, then the user's built-in
  // plugins that are switched on (Settings > Leet > Built-in plugins), read when it opens
  auto* tools = new QMenu(this);
  connect(tools, &QMenu::aboutToShow, this, [this, tools] {
    tools->clear();
    if (!m_session) {
      return;
    }
    const std::string root = utf8(qs((*m_session)->root()));
    tools->addAction(tr("INI Editor"), this, [this] { openDialog("inieditor"); });
    // crashlogtools: Skyrim SE only, as the plugin
    if (qs((*m_session)->game_name()) == "Skyrim Special Edition") {
      if (strata::builtins::builtin_enabled(root, "Crash Log Viewer")) {
        tools->addAction(tr("Crash Log Viewer"), this, [this] { openDialog("crashlogs"); });
      }
    }
  });
  ui->actionTool->setMenu(tools);
  if (auto* button = qobject_cast<QToolButton*>(ui->toolBar->widgetForAction(ui->actionTool))) {
    button->setPopupMode(QToolButton::InstantPopup);
  }
  // MainWindow::setupToolbar: a spacer before the toolbar's last separator, so the pinned
  // executables (loadExecutables puts them before that separator) sit on the right
  {
    QAction* last = nullptr;
    for (QAction* a : ui->toolBar->actions()) {
      if (a->isSeparator()) {
        last = a;
      }
    }
    if (last) {
      auto* spacer = new QWidget(ui->toolBar);
      spacer->setObjectName("toolBarSpacer");
      spacer->setSizePolicy(QSizePolicy::MinimumExpanding, QSizePolicy::Preferred);
      ui->toolBar->insertWidget(last, spacer);
    }
  }
  connect(ui->actionNotifications, &QAction::triggered, this, [this] { openDialog("problems"); });

  // the folder menu next to the profile (MainWindow::openFolderMenu)
  auto* folders = new QMenu(this);
  ui->openFolderMenu->setMenu(folders);
  connect(folders, &QMenu::aboutToShow, this, [this, folders] {
    folders->clear();
    if (!m_session) {
      return;
    }
    const auto f = (*m_session)->folders();
    const std::pair<QString, QString> entries[] = {
        {tr("Open Instance folder"), qs(f.instance)}, {tr("Open Mods folder"), qs(f.mods)},
        {tr("Open Profile folder"), qs(f.profile)},   {tr("Open Downloads folder"), qs(f.downloads)},
        {tr("Open Overwrite folder"), qs(f.overwrite)}, {tr("Open Game folder"), qs(f.game)}};
    for (const auto& [label, path] : entries) {
      folders->addAction(label, this, [this, path] { openFolder(path); });
    }
  });
}

bool MainWindow::openInstance(const QString& root, const QString& profile)
{
  try {
    m_session = strata::open_session(utf8(root), utf8(profile));
  } catch (const std::exception& e) {
    showError(QString::fromUtf8(e.what()));
    return false;
  }

  try {
    strata::remember_instance(utf8(root));
    if (m_installActions) {
      m_installActions();  // install area
    }
  } catch (const std::exception& e) {
    qWarning("cannot remember the instance: %s", e.what());
  }

  m_style.setInstance(root);
  m_appliedStyle.clear();  // another instance may hold a sheet of the same name
  applyInstanceStyle();

  setWindowTitle(QStringLiteral("%1 \u2013 Strata v%2")
                     .arg(qs((*m_session)->game_name()), qs(strata::version())));
  reloadAll();
  {
    // restoring the layout is no choice of "<Edit...>"
    QSignalBlocker block(ui->executablesListBox);
    m_layout->restore(**m_session);
    // no saved header (or one that couldn't be read): first-start widths (ColumnFit)
    if (!m_layout->restored(ui->modList->header())) {
      if (!m_modFit) {
        m_modFit = new ColumnFit(ui->modList, modlist::ColName, m_layout);
      }
      m_modFit->start();
    }
    m_fitPlugins = !m_layout->restored(ui->espList->header());
    ui->displayCategoriesBtn->setChecked(ui->categoriesGroup->isVisible());
    if (ui->executablesListBox->currentIndex() <= 0 && ui->executablesListBox->count() > 1) {
      ui->executablesListBox->setCurrentIndex(1);  // readSettings: never the "<Edit...>" entry
    }
  }
  m_oldExecutableIndex = ui->executablesListBox->currentIndex();
  if (ui->tabWidget->currentWidget() == ui->dataTab) {
    m_dataTab->activate();
  }
  qInfo("opened %s, profile \"%s\"", qUtf8Printable(root),
        qUtf8Printable(qs((*m_session)->profile_name())));
  return true;
}

void MainWindow::selectTab(int index)
{
  ui->tabWidget->setCurrentIndex(index);
}

void MainWindow::refresh()
{
  if (!m_session) {
    return;
  }
  try {
    m_session = strata::open_session((*m_session)->root(), (*m_session)->profile_name());
  } catch (const std::exception& e) {
    showError(QString::fromUtf8(e.what()));
    return;
  }
  m_layout->setSession(&**m_session);
  // another list: the conflict index is built again
  m_conflicts.reset();
  m_conflictsBusy = false;
  ++m_conflictsGeneration;
  reloadAll();
  qInfo("refreshed");
}

void MainWindow::toggleModForTest(const QString& name)
{
  if (auto* item = m_modByName.value(name)) {
    item->setCheckState(item->checkState() == Qt::Checked ? Qt::Unchecked : Qt::Checked);
  } else {
    qWarning("test hook: no mod \"%s\"", qUtf8Printable(name));
  }
}

void MainWindow::selectMod(const QString& name)
{
  if (auto* item = m_modByName.value(name)) {
    const QModelIndex index = m_modProxy.mapFromSource(item->index());
    ui->modList->selectionModel()->select(index, QItemSelectionModel::ClearAndSelect | QItemSelectionModel::Rows);
    ui->modList->selectionModel()->setCurrentIndex(index, QItemSelectionModel::NoUpdate);
    ui->modList->scrollTo(index);
  }
}

void MainWindow::showModColumn(int column)
{
  ui->modList->header()->setSectionHidden(column, false);
}

void MainWindow::dropOntoForTest(const QString& name, const QString& separator)
{
  auto* item = m_modByName.value(name);
  auto* sep  = m_modByName.value(separator);
  if (!item || !sep) {
    qWarning("test hook: no mod \"%s\" or separator \"%s\"", qUtf8Printable(name),
             qUtf8Printable(separator));
    return;
  }
  const QModelIndex index = m_modProxy.mapFromSource(item->index());
  ui->modList->selectionModel()->select(
      index, QItemSelectionModel::ClearAndSelect | QItemSelectionModel::Rows);
  onModDrop(m_modProxy.mapFromSource(sep->index()), -1, false);
}

void MainWindow::hideModColumn(int column)
{
  ui->modList->header()->setSectionHidden(column, true);
}

void MainWindow::showError(const QString& message)
{
  qCritical("%s", qUtf8Printable(message));
  m_statusText->setText(message);
}

void MainWindow::reloadAll()
{
  if (!m_session) {
    return;
  }
  loadProfiles();
  loadModList();
  m_pluginsTab->setSource(qs((*m_session)->root()), qs((*m_session)->profile_name()));
  loadExecutables();
  loadDownloads();
  m_dataTab->setSource(qs((*m_session)->root()), qs((*m_session)->profile_name()));
  m_rootTab->setSource(qs((*m_session)->root()), qs((*m_session)->profile_name()));
  if (ui->tabWidget->currentWidget() == ui->dataTab) {
    m_dataTab->activate();
  } else if (ui->tabWidget->currentWidget() == ui->rootTab) {
    m_rootTab->activate();
  }
  // StatusBar::updateNormalMessage: "game - instance - profile"; the folder in the tooltip
  const QString root = qs((*m_session)->root());
  m_statusText->setText(QStringLiteral("%1 - %2 - %3")
                            .arg(qs((*m_session)->game_name()), QFileInfo(root).fileName(),
                                 qs((*m_session)->profile_name())));
  m_statusText->setToolTip(root);
}

void MainWindow::loadProfiles()
{
  QSignalBlocker block(ui->profileBox);
  ui->profileBox->clear();
  ui->profileBox->addItem(tr("<Manage...>"));  // MainWindow::refreshProfiles
  try {
    for (const auto& name : (*m_session)->profile_names()) {
      ui->profileBox->addItem(qs(name));
    }
  } catch (const std::exception& e) {
    qWarning("cannot list the profiles: %s", e.what());
  }
  ui->profileBox->setCurrentText(qs((*m_session)->profile_name()));
}

void MainWindow::switchProfile(int comboIndex)
{
  if (!m_session) {
    return;
  }
  const QString current = qs((*m_session)->profile_name());
  if (comboIndex <= 0) {
    // "<Manage...>" opens the profiles dialog
    ui->profileBox->setCurrentText(current);
    openDialog("profiles");
    return;
  }
  const QString name = ui->profileBox->itemText(comboIndex);
  if (name == current) {
    return;
  }
  QString deployed;
  try {
    deployed = qs((*m_session)->deployed_profile());
  } catch (const std::exception& e) {
    qWarning("cannot ask the overlay daemon: %s", e.what());
  }
  if (deployed.isEmpty()) {
    selectProfile(name);
    return;
  }
  // deployed: strata_cli::select_profile unmounts (refusing while the game runs), selects
  // and mounts the new profile; on a worker, the unmount can take a moment
  const std::string root = utf8(qs((*m_session)->root()));
  const std::string wanted = utf8(name);
  ui->profileBox->setEnabled(false);
  QPointer<MainWindow> self(this);
  std::thread([self, root, wanted, name, current] {
    QString error;
    try {
      strata::modlist::profile_select(root, wanted);
    } catch (const std::exception& e) {
      error = QString::fromUtf8(e.what());
    }
    QMetaObject::invokeMethod(
        qApp,
        [self, error, name, current] {
          if (!self) {
            return;
          }
          self->ui->profileBox->setEnabled(true);
          if (!error.isEmpty()) {
            self->showError(tr("cannot switch to profile \"%1\": %2").arg(name, error));
            self->ui->profileBox->setCurrentText(current);
            return;
          }
          self->showProfile(name);
          qInfo("profile \"%s\" deployed", qUtf8Printable(name));
        },
        Qt::QueuedConnection);
  }).detach();
}

bool MainWindow::selectProfile(const QString& name)
{
  const QString current = qs((*m_session)->profile_name());
  try {
    // a fresh session: select_profile rewrites ModOrganizer.ini from what it read
    auto session = strata::open_session((*m_session)->root(), utf8(name));
    session->select_profile(utf8(name));
  } catch (const std::exception& e) {
    showError(QString::fromUtf8(e.what()));
    ui->profileBox->setCurrentText(current);
    return false;
  }
  return showProfile(name);
}

bool MainWindow::showProfile(const QString& name)
{
  try {
    m_session = strata::open_session((*m_session)->root(), utf8(name));
  } catch (const std::exception& e) {
    showError(QString::fromUtf8(e.what()));
    return false;
  }
  m_layout->setSession(&**m_session);
  // another list: the conflict index is built again
  m_conflicts.reset();
  m_conflictsBusy = false;
  ++m_conflictsGeneration;
  reloadAll();
  qInfo("profile \"%s\"", qUtf8Printable(name));
  return true;
}

void MainWindow::backupOrRestore(bool mods, bool restore)
{
  if (!m_session) {
    return;
  }
  try {
    if (!restore) {
      if (mods) {
        qInfo("backup %s", qUtf8Printable(qs((*m_session)->backup_mod_list())));
      } else {
        qInfo("backed up %zu load order files", (*m_session)->backup_plugins());
      }
      return;
    }
    // MainWindow::queryRestore: the backups, newest first, dates as dates
    QStringList suffixes, labels;
    for (const auto& s : (*m_session)->backups(mods)) {
      const QString suffix = qs(s);
      const QDateTime time = QDateTime::fromString(suffix, "yyyy_MM_dd_hh_mm_ss");
      suffixes << suffix;
      labels << (time.isValid() ? QLocale().toString(time, QLocale::ShortFormat) : suffix);
    }
    if (suffixes.isEmpty()) {
      QMessageBox::information(this, tr("No Backups"), tr("There are no backups to restore"));
      return;
    }
    bool ok             = false;
    const QString label = QInputDialog::getItem(this, tr("Choose backup to restore"), tr("Backup"),
                                                labels, 0, false, &ok);
    const qsizetype i = labels.indexOf(label);
    if (!ok || i < 0) {
      return;
    }
    const QByteArray suffix = suffixes[i].toUtf8();
    (*m_session)->restore_backup(mods, rust::Str(suffix.data(), std::size_t(suffix.size())));
  } catch (const std::exception& e) {
    showError(tr("backup: %1").arg(QString::fromUtf8(e.what())));
    return;
  }
  if (mods) {
    ++m_modGeneration;
    rebuildModList();
    refreshDeploy();
    updateConflicts();
  }
  m_pluginsTab->reload();
}

// ModListViewActions::checkModsForUpdates for every mod: `strata nexus check-updates` in
// its own process (the window never waits for Nexus); the list is read again after it.
void MainWindow::checkForUpdates()
{
  if (!m_session || (m_nexusCheck && m_nexusCheck->state() != QProcess::NotRunning)) {
    return;
  }
  QString program = QCoreApplication::applicationFilePath();
  if (program.endsWith(" (deleted)")) {
    program.chop(int(qstrlen(" (deleted)")));
  }
  if (!m_nexusCheck) {
    m_nexusCheck = new QProcess(this);
    connect(m_nexusCheck, &QProcess::finished, this, [this](int code, QProcess::ExitStatus status) {
      for (const QByteArray& line : m_nexusCheck->readAllStandardOutput().split('\n')) {
        if (!line.trimmed().isEmpty()) {
          qInfo("%s", line.constData());
        }
      }
      const QByteArray errors = m_nexusCheck->readAllStandardError().trimmed();
      if (status != QProcess::NormalExit || code != 0) {
        // MO2: "You are not currently authenticated with Nexus. Please do so under Settings -> Nexus."
        showError(tr("update check failed: %1").arg(QString::fromUtf8(errors)));
        return;
      }
      m_statusText->setText(tr("update check done"));
      refresh();
    });
  }
  m_statusText->setText(tr("checking Nexus for updates..."));
  qInfo("checking Nexus for updates");
  m_nexusCheck->start(program, {"nexus", "check-updates", "-i", qs((*m_session)->root())});
}

void MainWindow::showInstanceManager()
{
  const QString current = m_session ? qs((*m_session)->root()) : QString();
  const QString chosen  = strata::instances_ui::showInstanceManager(this, current);
  if (!chosen.isEmpty()) {
    switchInstance(chosen);
  }
}

void MainWindow::switchInstance(const QString& root)
{
  if (!m_session) {
    openInstance(root, QString());  // nothing open: no restart needed
    return;
  }
  if (m_run && m_run->state() != QProcess::NotRunning) {
    QMessageBox::information(this, tr("Switching instances"),
                             tr("A program started from Strata is still running. End it first."));
    return;
  }
  // InstanceManagerDialog::confirmSwitch
  const auto answer = QMessageBox::question(
      this, tr("Switching instances"),
      tr("Strata must restart to manage the instance '%1'.").arg(QFileInfo(root).fileName()),
      QMessageBox::Ok | QMessageBox::Cancel, QMessageBox::Ok);
  if (answer != QMessageBox::Ok) {
    return;
  }
  try {
    strata::remember_instance(utf8(root));
  } catch (const std::exception& e) {
    qWarning("cannot remember the instance: %s", e.what());
  }
  QString program = QCoreApplication::applicationFilePath();
  if (program.endsWith(" (deleted)")) {
    program.chop(int(qstrlen(" (deleted)")));  // rebuilt while open (strata_core::dirs::own_exe)
  }
  if (!QProcess::startDetached(program, {"-i", root})) {
    showError(tr("Cannot start %1").arg(program));
    return;
  }
  close();  // the layout is flushed as on a user's close
  QCoreApplication::quit();
}

void MainWindow::openDialog(const QString& name)
{
  if (name == "instances") {
    showInstanceManager();
    return;
  }
  if (name == "create-instance") {
    QString created;
    const QString to = strata::instances_ui::showCreateInstance(this, &created);
    if (!to.isEmpty()) {
      switchInstance(to);
    }
    return;
  }
  if (!m_session) {
    return;
  }
  const QString root    = qs((*m_session)->root());
  const QString profile = qs((*m_session)->profile_name());
  if (!qEnvironmentVariableIsEmpty("STRATA_SHELL_DIALOG_SCREENSHOT")) {
    strata::modlist::openDialogByName(this, name, root, profile);
    return;
  }
  QString selected;
  bool changed = false;
  if (name == "profiles") {
    changed = strata::modlist::showProfilesDialog(this, root, &selected);
  } else if (name == "executables") {
    changed = strata::modlist::showExecutablesDialog(this, root, profile);
  } else if (name == "settings") {
    changed = strata::modlist::showSettingsDialog(this, root);
  } else if (name == "inieditor") {
    strata::modlist::showIniEditor(this, root, profile);  // the profile's inis: no list shows them
  } else if (name == "problems") {
    // a fix changes plugins.txt or the executables
    if (strata::modlist::showProblemsDialog(this, root, profile)) {
      m_pluginsTab->reload();
      loadExecutables();
    }
  } else if (name == "crashlogs") {
    strata::modlist::showCrashLogViewer(this, root, profile);
  }
  if (!selected.isEmpty() && selected != profile) {
    openInstance(root, selected);  // the profile is selected (and a deploy remounted)
  } else if (changed) {
    refresh();
    if (name == "settings") {
      applyInstanceStyle();  // MO2 applies a new style when the dialog is closed with OK
    }
  }
}

// The instance's style (STRATA_STYLE overrides it: a test hook, and a quick way to try one);
// vs15 Dark when the sheet is not found
void MainWindow::applyInstanceStyle()
{
  const QString wanted = qEnvironmentVariable("STRATA_STYLE", qs((*m_session)->style()));
  if (wanted == m_appliedStyle) {
    return;
  }
  m_appliedStyle = wanted;
  if (!m_style.apply(wanted)) {
    m_style.apply("vs15 Dark.qss");
  }
}

void MainWindow::onModSortChanged(int column, Qt::SortOrder order)
{
  // ModListView::updateGroupByProxy: separators only nest by ascending priority
  const bool nested = column == modlist::ColPriority && order == Qt::AscendingOrder;
  if (nested != m_modNested) {
    m_modNested = nested;
    showModList();  // the same rows, nested or flat: no need to read them again
  }
}

namespace
{

bool truthy(const strata::StoredValue& v)
{
  const QByteArray t =
      QByteArray(reinterpret_cast<const char*>(v.bytes.data()), qsizetype(v.bytes.size()))
          .trimmed()
          .toLower();
  return v.found && !(t.isEmpty() || t == "0" || t == "false");
}

}  // namespace

void MainWindow::loadModList()
{
  // Everything that reads the disk happens on a worker with its own session (the UI thread
  // never waits for 1,362 meta.ini files); the result comes back as one piece.
  const int generation = ++m_modGeneration;
  const std::string root    = utf8(qs((*m_session)->root()));
  const std::string profile = utf8(qs((*m_session)->profile_name()));
  QPointer<MainWindow> self(this);
  std::thread([self, generation, root, profile] {
    auto data = std::make_shared<ModListData>();
    QElapsedTimer timer;
    timer.start();
    try {
      auto session = strata::open_session(root, profile);
      data->rows   = session->mods();
      
      data->alwaysShowSize =
          truthy(session->load_setting("Plugins", "Sub-Separator/alwaysShowSize"));
      // [Plugins] Sub-Separator\indentation: negative is Qt's default, 0 (MO2's default)
      // draws the arrow in the name column instead
      const auto indent = session->load_setting("Plugins", "Sub-Separator/indentation");
      data->indentation =
          indent.found ? QByteArray(reinterpret_cast<const char*>(indent.bytes.data()),
                                    qsizetype(indent.bytes.size()))
                             .trimmed()
                             .toInt()
                       : 0;
      data->displayForeign = session->display_foreign();
      data->invertAlt =
          truthy(session->load_setting("Plugins", "Sub-Separator/invertAltModifierWhenSeparatorIsCollapsed"));
      // MO2's defaults: all on
      auto on = [&](const char* key) {
        const auto v = session->load_setting("Settings", key);
        return !v.found || truthy(v);
      };
      data->rollUps = {on("collapsible_separators_icons_2"), on("collapsible_separators_icons_1"),
                       on("collapsible_separators_icons_3"), on("collapsible_separators_conflicts_to"),
                       on("collapsible_separators_conflicts_from")};
      data->game           = qs(session->game_name());
      const auto expanded = session->load_string_list("Widgets", "MainWindow_modList_index");
      if (expanded.found) {
        QStringList names;
        for (const auto& name : expanded.items) {
          names << qs(name);
        }
        data->expanded = names;
      }
      // this session read every meta.ini: it becomes the window's session
      data->session = std::make_shared<rust::Box<strata::Session>>(std::move(session));
    } catch (const std::exception& e) {
      data->error = QString::fromUtf8(e.what());
    }
    data->readMs = timer.elapsed();
    QMetaObject::invokeMethod(
        qApp,
        [self, generation, data] {
          if (self && generation == self->m_modGeneration) {
            if (data->session) {
              self->m_session = std::move(*data->session);
              data->session.reset();
              self->m_layout->setSession(&**self->m_session);
            }
            self->m_modData = data;
            self->showModList();
          }
        },
        Qt::QueuedConnection);
  }).detach();
}

void MainWindow::showModList()
{
  if (!m_modData) {
    return;
  }
  const ModListData& data = *m_modData;
  if (!data.error.isEmpty()) {
    showError(data.error);
    return;
  }
  QElapsedTimer timer;
  timer.start();

  if (data.indentation < 0) {
    ui->modList->resetIndentation();
  } else {
    ui->modList->setIndentation(data.indentation);
  }
  delete ui->modList->itemDelegate();
  m_rollUps = data.rollUps;
  ui->modList->setItemDelegate(new ModListDelegate(ui->modList, data.alwaysShowSize, &m_markers, &m_rollUps));
  m_modProxy.setDisplayForeign(data.displayForeign);
  ui->modList->setInvertAltOnCollapsed(data.invertAlt);

  // what the user had selected and where the list was scrolled, by name
  const QStringList selected = selectedMods();
  const QString current      = ui->modList->currentIndex().data(modlist::InternalNameRole).toString();
  const int scroll           = ui->modList->verticalScrollBar()->value();

  QStandardItemModel* next = m_modModel == &m_modModels[0] ? &m_modModels[1] : &m_modModels[0];
  QHash<QString, QStandardItem*> byName;
  {
    QSignalBlocker block(next);
    m_updatingMods = true;
    modlist::fill(*next, data.rows, m_modNested, data.game, &byName);
    m_updatingMods = false;
  }
  {
    // the swap resets the view: keep the header as it is (hidden columns, sizes, sort)
    LayoutState::Quiet quiet(*m_layout);
    const QByteArray header = ui->modList->header()->saveState();
    m_modProxy.setSourceModel(next);
    ui->modList->header()->restoreState(header);
  }
  m_modModel->removeRows(0, m_modModel->rowCount());
  m_modModel  = next;
  m_modByName = std::move(byName);

  if (!m_expandedLoaded && m_modNested) {
    // WidgetSettings::restoreTreeExpandState: the open ones by name; without the key every
    // separator is open (refreshExpandedItems with nothing collapsed)
    m_expandedLoaded = true;
    m_expanded.clear();
    if (data.expanded) {
      for (const auto& name : *data.expanded) {
        m_expanded.insert(name);
      }
    } else {
      std::vector<QStandardItem*> stack;
      for (int r = 0; r < m_modModel->rowCount(); ++r) {
        stack.push_back(m_modModel->item(r));
      }
      while (!stack.empty()) {
        auto* item = stack.back();
        stack.pop_back();
        if (item->hasChildren()) {
          m_expanded.insert(item->text());
          for (int c = 0; c < item->rowCount(); ++c) {
            stack.push_back(item->child(c));
          }
        }
      }
    }
  }
  applyExpandState();
  updateModCounter();
  if (!selected.isEmpty()) {
    QItemSelection selection;
    for (const QString& name : selected) {
      if (auto* item = m_modByName.value(name)) {
        const QModelIndex index = m_modProxy.mapFromSource(item->index());
        selection.select(index, index.siblingAtColumn(modlist::ColumnCount - 1));
      }
    }
    ui->modList->selectionModel()->select(selection, QItemSelectionModel::ClearAndSelect);
  }
  if (auto* item = m_modByName.value(current)) {
    ui->modList->selectionModel()->setCurrentIndex(m_modProxy.mapFromSource(item->index()),
                                                   QItemSelectionModel::NoUpdate);
  }
  ui->modList->verticalScrollBar()->setValue(scroll);
  if (!m_selectAfterLoad.isEmpty() && m_modByName.contains(m_selectAfterLoad)) {
    selectMod(std::exchange(m_selectAfterLoad, QString()));
  }
  if (m_conflicts) {
    applyAllConflicts();
  } else if (!m_conflictsBusy) {
    buildConflicts();
  }
  m_filterPanel->setEntries((*m_session)->filter_entries());
  applyCriteria();
  qDebug("mod list: %zu rows, read in %lld ms (worker), shown in %lld ms", data.rows.size(),
         data.readMs, timer.elapsed());
}

void MainWindow::applyExpandState()
{
  if (!m_modNested) {
    return;
  }
  m_applyingExpand = true;
  ui->modList->collapseAll();
  std::vector<QModelIndex> stack{QModelIndex()};
  while (!stack.empty()) {
    const QModelIndex parent = stack.back();
    stack.pop_back();
    for (int r = 0; r < m_modProxy.rowCount(parent); ++r) {
      const QModelIndex index = m_modProxy.index(r, 0, parent);
      if (m_modProxy.hasChildren(index)) {
        if (m_expanded.contains(index.data(Qt::DisplayRole).toString())) {
          ui->modList->expand(index);
        }
        stack.push_back(index);
      }
    }
  }
  m_applyingExpand = false;
}


QStringList MainWindow::selectedMods() const
{
  QStringList names;
  if (!ui->modList->selectionModel()) {
    return names;
  }
  for (const QModelIndex& index : ui->modList->selectionModel()->selectedRows(0)) {
    const int kind = index.data(modlist::KindRole).toInt();
    if (kind != modlist::Overwrite) {
      names << index.data(modlist::InternalNameRole).toString();
    }
  }
  return names;
}

template <class Op>
void MainWindow::modOperation(const char* what, Op op)
{
  if (!m_session) {
    return;
  }
  ++m_modGeneration;  // a list still being read on a worker is out of date now
  try {
    applyModEvents(op(**m_session));
  } catch (const std::exception& e) {
    showError(QStringLiteral("%1: %2").arg(QString::fromUtf8(what), QString::fromUtf8(e.what())));
    rebuildModList();  // show what the core has, whatever the model did
  }
}

void MainWindow::applyModEvents(const rust::Vec<strata::ModEvent>& events)
{
  bool rebuild = false;
  bool layers  = false;
  std::optional<rust::Vec<strata::ModRow>> rows;
  auto rowOf = [&](const QString& name) -> const strata::ModRow* {
    if (!rows) {
      rows = (*m_session)->mods();  // from the session's cache: no disk
    }
    for (const auto& r : *rows) {
      if (qs(r.name) == name) {
        return &r;
      }
    }
    return nullptr;
  };

  for (const auto& e : events) {
    switch (e.kind) {
    case 0: {  // state: only the check boxes change
      m_updatingMods = true;
      for (const auto& name : e.names) {
        if (auto* item = m_modByName.value(qs(name))) {
          item->setCheckState(e.enabled ? Qt::Checked : Qt::Unchecked);
        }
      }
      m_updatingMods = false;
      layers = true;
      break;
    }
    case 5:    // meta.ini: that row in place
    case 6: {  // overwrite
      const QString name = e.kind == 6 ? QStringLiteral("Overwrite") : qs(e.name);
      auto* item         = m_modByName.value(name);
      const auto* row    = rowOf(name);
      if (item && row && m_modData) {
        m_updatingMods = true;
        modlist::update(item, *row, m_modData->game);
        m_updatingMods = false;
      }
      break;
    }
    default:  // moved, nest, renamed, added: the tree changes
      rebuild = true;
      layers  = true;
      break;
    }
  }
  if (rebuild) {
    rebuildModList();
  } else {
    applyCriteria();
    updateModCounter();
  }
  if (layers) {
    refreshDeploy();
    updateConflicts();
    m_pluginsTab->reload();  // other plugins are available now
  }
}

void MainWindow::rebuildModList()
{
  if (!m_session || !m_modData) {
    return;
  }
  m_modData->rows = (*m_session)->mods();
  showModList();
}

void MainWindow::onModItemChanged(QStandardItem* item)
{
  if (m_updatingMods || item->column() != modlist::ColName || !item->isCheckable()) {
    return;
  }
  const QString name  = item->data(modlist::InternalNameRole).toString();
  const bool enabled  = item->checkState() == Qt::Checked;
  bool changed        = false;
  modOperation("enable", [&](strata::Session& s) {
    auto events = s.set_enabled(rustNames({name}), enabled);
    changed     = !events.empty();
    return events;
  });
  if (!changed) {
    // the core refused (or nothing to do): the box shows the core's state again
    m_updatingMods = true;
    item->setCheckState(enabled ? Qt::Unchecked : Qt::Checked);
    m_updatingMods = false;
  }
}

std::optional<MainWindow::DropTarget> MainWindow::dropTarget(const QModelIndex& parent, int row,
                                                             bool belowExpandedSeparator)
{
  DropTarget t;
  t.nested = m_modNested;
  t.below  = belowExpandedSeparator;
  if (m_modNested) {
    // ModListByPriorityProxy::dropMimeData: the source model has the core's tree rows
    const QModelIndex sourceParent = m_modProxy.mapToSource(parent);
    t.parent                       = sourceParent.data(modlist::InternalNameRole).toString();
    if (row >= 0) {
      if (row < m_modProxy.rowCount(parent)) {
        t.sourceRow = m_modProxy.mapToSource(m_modProxy.index(row, 0, parent)).row();
      } else if (!sourceParent.isValid()) {
        t.sourceRow = m_modModel->rowCount() - 1;  // the end of the list: before Overwrite
      } else {
        t.sourceRow = m_modModel->itemFromIndex(sourceParent)->rowCount();
      }
    }
    return t;
  }
  // a flat list (ModList::dropMimeData): only while sorted by priority
  const auto* header = ui->modList->header();
  if (header->sortIndicatorSection() != modlist::ColPriority) {
    showError(tr("Drag&Drop is only supported when sorting by priority"));
    return std::nullopt;
  }
  const bool ascending = header->sortIndicatorOrder() == Qt::AscendingOrder;
  t.priority           = ascending ? -1 : 0;  // dropped past the last row
  if (row >= 0 && row < m_modProxy.rowCount(parent)) {
    const int p = m_modProxy.index(row, 0, parent).data(modlist::PriorityRole).toInt();
    t.priority  = ascending ? p : p + 1;
  }
  return t;
}

rust::Vec<strata::ModEvent> MainWindow::applyDrop(strata::Session& s, const QStringList& names,
                                                  const DropTarget& t)
{
  if (!t.nested) {
    return s.move_mods(rustNames(names), t.priority);
  }
  const QByteArray p = t.parent.toUtf8();
  return s.drop_mods(rustNames(names), rust::Str(p.data(), std::size_t(p.size())), t.sourceRow, t.below);
}

void MainWindow::onModDrop(const QModelIndex& parent, int row, bool belowExpandedSeparator)
{
  const QStringList names = selectedMods();
  if (names.isEmpty() || !m_session) {
    return;
  }
  if (const auto target = dropTarget(parent, row, belowExpandedSeparator)) {
    modOperation("drop", [&](strata::Session& s) { return applyDrop(s, names, *target); });
  }
}

void MainWindow::onForeignDrop(const QList<QUrl>& urls, const QString& download,
                               const QModelIndex& parent, int row, bool belowExpandedSeparator)
{
  if (!m_session) {
    return;
  }
  const auto target = dropTarget(parent, row, belowExpandedSeparator);
  if (!target) {
    return;
  }
  // ModListDropInfo: one archive or one folder from outside the instance, or a download
  QString path;
  if (!download.isEmpty()) {
    path = qs((*m_session)->folders().downloads) + "/" + download;
  } else if (urls.size() == 1 && urls[0].isLocalFile()) {
    path = QFileInfo(urls[0].toLocalFile()).absoluteFilePath();
  }
  const auto f = (*m_session)->folders();
  const QString canonical = QFileInfo(path).canonicalFilePath();
  for (const QString& inside : {qs(f.mods), qs(f.overwrite)}) {
    const QString dir = QFileInfo(inside).canonicalFilePath();
    if (!canonical.isEmpty() && !dir.isEmpty() && (canonical + "/").startsWith(dir + "/")) {
      // ModList::dropLocalFiles moves files between mods
      showError(tr("Moving files between mods by drag and drop is not in Strata yet."));
      return;
    }
  }
  const QFileInfo info(path);
  if (path.isEmpty() || !info.exists()) {
    showError(tr("Drop one archive or one folder at a time."));
    return;
  }
  bool archive = false;
  for (const auto& e : strata::archive_extensions()) {
    archive = archive || info.suffix().compare(qs(e), Qt::CaseInsensitive) == 0;
  }
  if (!info.isDir() && !archive) {
    showError(tr("%1 is not an archive Strata can install.").arg(info.fileName()));
    return;
  }
  // MO2 queues it: the drop ends first, so an installer dialog can take drops itself
  const DropTarget t = *target;
  QTimer::singleShot(0, this, [this, path, t, isDir = info.isDir()] {
    if (isDir) {
      copyFolderToMod(path, t);
    } else {
      installDropped(path, t);
    }
  });
}

void MainWindow::copyFolderToMod(const QString& folder, const DropTarget& target)
{
  // ModListView::onExternalFolderDropped: the user names the new mod, the folder's content
  // is copied into it, the mod goes where it was dropped
  const QString base = QFileInfo(folder).fileName();
  QString name       = base;
  for (;;) {
    bool ok = false;
    name    = QInputDialog::getText(this, tr("Copy Folder..."),
                                    tr("This will copy the content of %1 to a new mod.\n"
                                       "Please enter the name:").arg(base),
                                    QLineEdit::Normal, name, &ok);
    if (!ok) {
      return;
    }
    if (!name.trimmed().isEmpty()) {
      break;
    }
  }
  if (m_modByName.contains(name.trimmed())) {
    showError(tr("A mod with this name already exists."));
    return;
  }
  const QByteArray n = name.trimmed().toUtf8();
  const QByteArray from = folder.toUtf8();
  modOperation("copy folder", [&](strata::Session& s) {
    auto events = s.create_mod_from_folder(rust::Str(n.data(), std::size_t(n.size())),
                                           rust::Str(from.data(), std::size_t(from.size())));
    for (auto& e : applyDrop(s, {name.trimmed()}, target)) {
      events.push_back(std::move(e));
    }
    return events;
  });
  selectMod(name.trimmed());
}

void MainWindow::installDropped(const QString& archive, const DropTarget& target)
{
  const QString root    = qs((*m_session)->root());
  const QString profile = qs((*m_session)->profile_name());
  QPointer<MainWindow> self(this);
  strata::startInstall(this, root, profile, archive, QString(),
                       [self, root, profile, archive, target](const QString& report, const QString& error) {
    if (!self) {
      return;
    }
    if (!error.isEmpty()) {
      if (!error.contains("cancelled")) {
        self->showError(tr("installing %1: %2").arg(QFileInfo(archive).fileName(), error));
      }
      emit self->dropInstallFinished(QString());
      return;
    }
    const QString name =
        QJsonDocument::fromJson(report.toUtf8()).object()["installed"].toObject()["name"].toString();
    qInfo("%s: installed as '%s'", qUtf8Printable(QFileInfo(archive).fileName()), qUtf8Printable(name));
    try {
      // the installer wrote the list from its own session: the drop goes through a new one
      auto session = strata::open_session(utf8(root), utf8(profile));
      self->applyDrop(*session, {name}, target);
    } catch (const std::exception& e) {
      self->showError(tr("cannot move %1 where it was dropped: %2").arg(name, QString::fromUtf8(e.what())));
    }
    self->refresh();
    self->m_selectAfterLoad = name;
    emit self->dropInstallFinished(name);
  });
}

void MainWindow::showModContextMenu(const QPoint& pos)
{
  if (!m_session) {
    return;
  }
  const QModelIndex clicked = ui->modList->indexAt(pos).siblingAtColumn(0);
  const QStringList names   = selectedMods();
  const int kind            = clicked.data(modlist::KindRole).toInt();
  const QString name        = clicked.data(modlist::InternalNameRole).toString();
  QMenu menu(this);

  if (kind == modlist::Overwrite) {
    addOverwriteActions(menu);
    menu.exec(ui->modList->viewport()->mapToGlobal(pos));
    return;
  }

  if (!clicked.isValid()) {
    // ModListView: the global menu on the empty part of the list
    fillGlobalModMenu(menu, {});
    menu.exec(ui->modList->viewport()->mapToGlobal(pos));
    return;
  }

  // ModListContextMenu: "All Mods", collapse / expand for a separator with mods, then the
  // actions the core has, "Information..." last (the default action)
  auto* allMods = menu.addMenu(tr("All Mods"));
  fillGlobalModMenu(*allMods, clicked);
  if (ui->modList->model()->hasChildren(clicked)) {
    const bool expanded = ui->modList->isExpanded(clicked);
    menu.addSeparator();
    menu.addAction(tr("Collapse all"), ui->modList, &QTreeView::collapseAll);
    menu.addAction(tr("Collapse others"), this, [this, clicked, expanded] {
      ui->modList->collapseAll();
      ui->modList->setExpanded(clicked, expanded);
    });
    menu.addAction(tr("Expand all"), ui->modList, &QTreeView::expandAll);
  }
  menu.addSeparator();

  if (!names.isEmpty()) {
    addSelectionActions(menu, names);
  }

  if (clicked.isValid() && kind != modlist::Overwrite && kind != modlist::Foreign) {
    addModActions(menu, clicked, names);
  }
  if (clicked.isValid() && kind != modlist::Foreign) {
    menu.addSeparator();
    menu.addAction(tr("Open in file manager"), this, [this, name] {
      const QByteArray n = name.toUtf8();
      openFolder(qs((*m_session)->mod_path(rust::Str(n.data(), std::size_t(n.size())))));
    });
  }
  if (clicked.isValid() && kind != modlist::Foreign && kind != modlist::Separator) {
    menu.addSeparator();
    menu.setDefaultAction(menu.addAction(tr("Information..."), this, [this, name] { openModInfo(name); }));
  }
  menu.exec(ui->modList->viewport()->mapToGlobal(pos));
}

// ModListContextMenu::addOverwriteActions: the moves only when Overwrite holds something.
void MainWindow::addOverwriteActions(QMenu& menu)
{
  const QString dir = qs((*m_session)->folders().overwrite);
  if (QDir(dir).count() > 2) {
    menu.addAction(tr("Sync to Mods..."), this, [this] {
      if (listtools::syncOverwrite(this, **m_session)) {
        refresh();  // OrganizerCore::syncOverwrite: refreshDirectoryStructure
        refreshDeploy();
      }
    });
    menu.addAction(tr("Create Mod..."), this, [this] {
      bool ok = false;
      const QString name = QInputDialog::getText(this, tr("Create Mod..."),
                                                 tr("This will move all files from overwrite into a new, regular mod.\n"
                                                    "Please enter a name:"),
                                                 QLineEdit::Normal, QString(), &ok);
      if (!ok || name.trimmed().isEmpty()) {
        return;
      }
      if (m_modByName.contains(name.trimmed())) {
        showError(tr("A mod with this name already exists"));
        return;
      }
      const QByteArray n = name.trimmed().toUtf8();
      modOperation("create mod from overwrite", [&](strata::Session& s) {
        return s.move_overwrite(rust::Str(n.data(), std::size_t(n.size())), true);
      });
      updateConflicts();
    });
    menu.addAction(tr("Move content to Mod..."), this, [this] {
      QStringList mods;
      for (const auto& row : (*m_session)->mods()) {
        if (row.kind == 0 /* regular */) {
          mods << qs(row.name);
        }
      }
      bool ok            = false;
      const QString mod = QInputDialog::getItem(this, tr("Select a mod..."), tr("Mod"), mods, 0, false, &ok);
      if (!ok || mod.isEmpty()) {
        return;
      }
      const QByteArray n = mod.toUtf8();
      modOperation("move overwrite", [&](strata::Session& s) {
        return s.move_overwrite(rust::Str(n.data(), std::size_t(n.size())), false);
      });
      updateConflicts();
      refreshDeploy();
    });
    menu.addAction(tr("Clear Overwrite..."), this, [this, dir] {
      if (QMessageBox::question(this, tr("Are you sure?"),
                                tr("About to recursively delete:\n") + dir + "\n" + tr("(to the trash)"),
                                QMessageBox::Ok | QMessageBox::Cancel) != QMessageBox::Ok) {
        return;
      }
      modOperation("clear overwrite", [&](strata::Session& s) { return s.clear_overwrite(true); });
      updateConflicts();
    });
  }
  menu.addAction(tr("Open in file manager"), this, [this, dir] { openFolder(dir); });
}

// The selection's actions: enable, disable, Send to... (ModListContextMenu::addSendToContextMenu).
void MainWindow::addSelectionActions(QMenu& menu, const QStringList& names)
{
  menu.addAction(tr("Enable selected"), this, [this, names] {
    modOperation("enable", [&](strata::Session& s) { return s.set_enabled(rustNames(names), true); });
  });
  menu.addAction(tr("Disable selected"), this, [this, names] {
    modOperation("disable", [&](strata::Session& s) { return s.set_enabled(rustNames(names), false); });
  });
  menu.addSeparator();
  QMenu* send = menu.addMenu(tr("Send to... "));
  send->addAction(tr("Lowest priority"), this, [this, names] {
    modOperation("send to top", [&](strata::Session& s) { return s.move_mods(rustNames(names), 0); });
  });
  send->addAction(tr("Highest priority"), this, [this, names] {
    modOperation("send to bottom", [&](strata::Session& s) { return s.move_mods(rustNames(names), -1); });
  });
  send->addAction(tr("Priority..."), this, [this, names] {
    bool ok        = false;
    const int max  = m_modByName.size();
    const int to   = QInputDialog::getInt(this, tr("Set Priority"), tr("Set the priority of the selected mods"),
                                          0, 0, max, 1, &ok);
    if (ok) {
      modOperation("send to priority", [&](strata::Session& s) { return s.move_mods(rustNames(names), to); });
    }
  });
  send->addAction(tr("Separator..."), this, [this, names] {
    QStringList separators;
    for (auto it = m_modByName.cbegin(); it != m_modByName.cend(); ++it) {
      if (it.value()->data(modlist::KindRole).toInt() == modlist::Separator) {
        separators << it.key();
      }
    }
    separators.sort(Qt::CaseInsensitive);
    QStringList labels;
    for (const QString& s : separators) {
      labels << QString(s).remove("_separator");
    }
    bool ok             = false;
    const QString label = QInputDialog::getItem(this, tr("Select a separator..."), tr("Separator"),
                                                labels, 0, false, &ok);
    const qsizetype i = labels.indexOf(label);
    if (ok && i >= 0) {
      const QByteArray sep = separators[i].toUtf8();
      modOperation("send to separator", [&](strata::Session& s) {
        return s.send_to_separator(rustNames(names), rust::Str(sep.data(), std::size_t(sep.size())));
      });
    }
  });
}

// One mod's or separator's actions: rename, remove, notes, colours (addRegularActions,
// addSeparatorActions).
void MainWindow::addModActions(QMenu& menu, const QModelIndex& clicked, const QStringList& names)
{
  const int kind      = clicked.data(modlist::KindRole).toInt();
  const QString name  = clicked.data(modlist::InternalNameRole).toString();
  const QString shown = clicked.data(Qt::DisplayRole).toString();
  menu.addSeparator();
  if (names.size() <= 1 && kind != modlist::Backup) {
    // addSeparatorActions names them after what they are
    menu.addAction(kind == modlist::Separator ? tr("Rename Separator...") : tr("Rename Mod..."), this,
                   [this, name, shown, kind] {
      bool ok               = false;
      const QString newName = QInputDialog::getText(this, tr("Rename Mod"), tr("New name:"),
                                                    QLineEdit::Normal, shown, &ok);
      if (ok && !newName.trimmed().isEmpty() && newName != shown) {
        // a separator keeps its suffix (the core's makeInternalName)
        const QByteArray o = name.toUtf8();
        const QByteArray n = (kind == modlist::Separator ? newName.trimmed() + "_separator"
                                                         : newName.trimmed()).toUtf8();
        modOperation("rename", [&](strata::Session& s) {
          return s.rename_mod(rust::Str(o.data(), std::size_t(o.size())),
                              rust::Str(n.data(), std::size_t(n.size())));
        });
      }
    });
  }
  menu.addAction(kind == modlist::Separator ? tr("Remove Separator...") : tr("Remove Mod..."), this,
                 [this, names] { removeMods(names); });
  QAction* notes = menu.addAction(tr("Edit Notes..."), this, [this, name] {
    bool ok              = false;
    const QString before = m_modByName.value(name) ? m_modByName.value(name)->data(modlist::CommentsRole).toString()
                                                   : QString();
    const QString text = QInputDialog::getText(this, tr("Notes"), tr("Notes:"), QLineEdit::Normal, before, &ok);
    if (ok) {
      const QByteArray n = name.toUtf8();
      const QByteArray t = text.toUtf8();
      modOperation("notes", [&](strata::Session& s) {
        return s.set_comments(rust::Str(n.data(), std::size_t(n.size())), rust::Str(t.data(), std::size_t(t.size())));
      });
    }
  });
  notes->setVisible(kind != modlist::Separator);  // MO2's separator menu has no notes entry
  if (kind == modlist::Separator) {
    menu.addAction(tr("Select Color..."), this, [this, name] {
      const QColor color = QColorDialog::getColor(Qt::white, this, tr("Select Color..."), QColorDialog::ShowAlphaChannel);
      if (color.isValid()) {
        const QByteArray n = name.toUtf8();
        const std::uint32_t rgba = (std::uint32_t(color.red()) << 24) | (std::uint32_t(color.green()) << 16) |
                                   (std::uint32_t(color.blue()) << 8) | std::uint32_t(color.alpha());
        modOperation("colour", [&](strata::Session& s) {
          return s.set_color(rust::Str(n.data(), std::size_t(n.size())), true, rgba);
        });
      }
    });
    menu.addAction(tr("Reset Color"), this, [this, name] {
      const QByteArray n = name.toUtf8();
      modOperation("colour", [&](strata::Session& s) {
        return s.set_color(rust::Str(n.data(), std::size_t(n.size())), false, 0);
      });
    });
  }
}

void MainWindow::fillGlobalModMenu(QMenu& menu, const QModelIndex& clicked)
{
  // ModListGlobalContextMenu::populate
  const QString name = clicked.data(modlist::InternalNameRole).toString();
  const int kind     = clicked.data(modlist::KindRole).toInt();
  const auto* header = ui->modList->header();
  const bool byPriority = header->sortIndicatorSection() == modlist::ColPriority;
  const bool ascending  = header->sortIndicatorOrder() == Qt::AscendingOrder;
  auto askName = [this](const QString& title, const QString& text) {
    bool ok = false;
    const QString n = QInputDialog::getText(this, title, text, QLineEdit::Normal, QString(), &ok);
    return ok ? n.trimmed() : QString();
  };
  auto createMod = [this, askName](const QString& at) {
    const QString n = askName(tr("Create Mod..."), tr("This will create an empty mod.\nPlease enter a name:"));
    if (n.isEmpty()) {
      return;
    }
    if (m_modByName.contains(n)) {
      showError(tr("A mod with this name already exists"));
      return;
    }
    const QByteArray nn = n.toUtf8(), a = at.toUtf8();
    modOperation("create mod", [&](strata::Session& s) {
      return s.create_empty_mod(rust::Str(nn.data(), std::size_t(nn.size())), rust::Str(a.data(), std::size_t(a.size())));
    });
    selectMod(n);
  };
  auto createSeparator = [this, askName](const QString& at) {
    const QString n = askName(tr("Create Separator..."), tr("This will create a new separator.\nPlease enter a name:"));
    if (n.isEmpty()) {
      return;
    }
    const QByteArray t = n.toUtf8(), a = at.toUtf8();
    modOperation("create separator", [&](strata::Session& s) {
      return s.create_separator(rust::Str(t.data(), std::size_t(t.size())), rust::Str(a.data(), std::size_t(a.size())));
    });
  };
  auto installAt = [this](const QModelIndex& index, bool inside) {
    // the archive goes where a drop on that row would put it
    const auto target = inside ? dropTarget(index, -1, false) : dropTarget(index.parent(), index.row(), false);
    const QString file = QFileDialog::getOpenFileName(
        this, tr("Choose Mod"), qs((*m_session)->folders().downloads),
        tr("Mod Archive") + QStringLiteral(" (*.zip *.rar *.7z *.fomod *.001)"));
    if (target && !file.isEmpty()) {
      installDropped(file, *target);
    }
  };

  if (clicked.isValid() && byPriority && kind != modlist::Backup && kind != modlist::Overwrite &&
      kind != modlist::Foreign) {
    QString installText   = tr("Install mod above... ");
    QString createText    = tr("Create empty mod above");
    QString separatorText = tr("Create separator above");
    const bool separator  = kind == modlist::Separator;
    if (separator) {
      installText = tr("Install mod inside... ");
      createText  = tr("Create empty mod inside");
      if (ascending) {  // separators nest in ascending priority (Sub-Separator)
        separatorText = tr("Create separator inside");
      }
    } else if (!ascending) {
      installText = tr("Install mod below... ");
      createText  = tr("Create empty mod below");
    }
    const QPersistentModelIndex at(clicked);
    menu.addAction(installText, this, [installAt, at, separator] { installAt(at, separator); });
    menu.addAction(createText, this, [createMod, name] { createMod(name); });
    menu.addAction(separatorText, this, [createSeparator, name] { createSeparator(name); });
  } else {
    menu.addAction(tr("Install mod..."), ui->actionInstallMod, &QAction::trigger);
    menu.addAction(tr("Create empty mod"), this, [createMod] { createMod(QString()); });
    menu.addAction(tr("Create separator"), this, [createSeparator] { createSeparator(QString()); });
  }
  if (m_modNested) {
    menu.addSeparator();
    menu.addAction(tr("Collapse all"), ui->modList, &QTreeView::collapseAll);
    menu.addAction(tr("Expand all"), ui->modList, &QTreeView::expandAll);
  }
  menu.addSeparator();
  const bool filtered = !ui->modFilterEdit->text().isEmpty() || m_filterPanel->active();
  for (const bool on : {true, false}) {
    const QString text = on ? (filtered ? tr("Enable all matching mods") : tr("Enable all"))
                            : (filtered ? tr("Disable all matching mods") : tr("Disable all"));
    menu.addAction(text, this, [this, on] {
      // ModListViewActions::setAllMatchingModsEnabled: every mod the list shows
      QStringList names;
      std::vector<QModelIndex> stack{QModelIndex()};
      while (!stack.empty()) {
        const QModelIndex parent = stack.back();
        stack.pop_back();
        for (int r = 0; r < m_modProxy.rowCount(parent); ++r) {
          const QModelIndex index = m_modProxy.index(r, 0, parent);
          if (index.flags() & Qt::ItemIsUserCheckable) {
            names << index.data(modlist::InternalNameRole).toString();
          }
          stack.push_back(index);
        }
      }
      modOperation(on ? "enable all" : "disable all",
                   [&](strata::Session& s) { return s.set_enabled(rustNames(names), on); });
    });
  }
  menu.addAction(tr("Check for updates"), this, &MainWindow::checkForUpdates);
  menu.addAction(tr("Auto assign categories"), this, [this] {
    // ModListViewActions::assignCategories
    if (QMessageBox::question(this, tr("Are you sure?"),
                              tr("This action will remove any existing categories on any mod with a valid Nexus "
                                 "category mapping. Are you certain you want to proceed?"),
                              QMessageBox::Yes | QMessageBox::Cancel) != QMessageBox::Yes) {
      return;
    }
    try {
      const auto changed = (*m_session)->auto_assign_categories();
      qInfo("auto assign categories: %zu mods changed", changed.size());
    } catch (const std::exception& e) {
      showError(tr("auto assign categories: %1").arg(QString::fromUtf8(e.what())));
    }
    refresh();
  });
  menu.addAction(tr("Refresh"), this, &MainWindow::refresh);
  menu.addAction(tr("Export to csv..."), this, [this] {
    QStringList visible;
    std::vector<QModelIndex> stack{QModelIndex()};
    while (!stack.empty()) {
      const QModelIndex parent = stack.back();
      stack.pop_back();
      for (int r = 0; r < m_modProxy.rowCount(parent); ++r) {
        const QModelIndex index = m_modProxy.index(r, 0, parent);
        visible << index.data(modlist::InternalNameRole).toString();
        stack.push_back(index);
      }
    }
    listtools::exportCsv(this, **m_session, visible);
  });
}

void MainWindow::removeMods(const QStringList& names)
{
  if (names.isEmpty() || !m_session) {
    return;
  }
  // Leet change 8: trash, permanently or cancel, the answer remembered in [DialogChoices]
  const auto choice = strata::modlist::askDeleteChoice(
      this, qs((*m_session)->root()), tr("Confirm"),
      tr("Remove the following mods?<br><ul><li>%1</li></ul>").arg(names.join("</li><li>")),
      names.size() == 1 ? "removeMod" : "removeMods");
  if (choice == strata::modlist::DeleteChoice::Cancel) {
    return;
  }
  const bool trash = choice == strata::modlist::DeleteChoice::Trash;
  modOperation("remove", [&](strata::Session& s) { return s.remove_mods(rustNames(names), trash); });
}

void MainWindow::editSeparatorLevel(const QString& name)
{
  // SubSeparator::SeparatorDialog: "<name> - Sub-Separator", one spin box "Level"
  auto* item = m_modByName.value(name);
  if (!item || !m_session) {
    return;
  }
  QDialog dialog(this);
  dialog.setObjectName("SeparatorDialog");
  dialog.setWindowTitle(QString(name).remove(QRegularExpression("_separator$")) + " - Sub-Separator");
  auto* form  = new QFormLayout(&dialog);
  auto* level = new QSpinBox(&dialog);
  level->setValue(item->data(modlist::NestRole).toInt());
  form->addRow(tr("Level"), level);
  auto* buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel, Qt::Horizontal, &dialog);
  form->addRow(buttons);
  connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
  connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
  if (dialog.exec() != QDialog::Accepted) {
    return;
  }
  const QByteArray n = name.toUtf8();
  const auto nest    = std::uint32_t(std::max(0, level->value()));
  modOperation("separator level", [&](strata::Session& s) {
    return s.set_nest(rust::Str(n.data(), std::size_t(n.size())), nest);
  });
}

void MainWindow::buildConflicts()
{
  if (!m_session) {
    return;
  }
  m_conflictsBusy           = true;
  const int generation      = ++m_conflictsGeneration;
  const std::string root    = utf8(qs((*m_session)->root()));
  const std::string profile = utf8(qs((*m_session)->profile_name()));
  QPointer<MainWindow> self(this);
  std::thread([self, generation, root, profile] {
    std::shared_ptr<rust::Box<strata::Conflicts>> index;
    QString error;
    QElapsedTimer timer;
    timer.start();
    try {
      index = std::make_shared<rust::Box<strata::Conflicts>>(strata::open_session(root, profile)->build_conflicts());
    } catch (const std::exception& e) {
      error = QString::fromUtf8(e.what());
    }
    const qint64 ms = timer.elapsed();
    QMetaObject::invokeMethod(
        qApp,
        [self, generation, index, error, ms] {
          if (!self || generation != self->m_conflictsGeneration) {
            return;
          }
          self->m_conflictsBusy = false;
          if (!index) {
            self->showError(tr("cannot build the conflict index: %1").arg(error));
            return;
          }
          self->m_conflicts = index;
          qDebug("conflict index built in %lld ms", ms);
          self->applyAllConflicts();
          self->applyCriteria();
          if (self->m_conflictsAgain) {
            self->m_conflictsAgain = false;
            self->updateConflicts();
          }
        },
        Qt::QueuedConnection);
  }).detach();
}

void MainWindow::updateConflicts()
{
  if (!m_session) {
    return;
  }
  if (m_conflictsBusy || !m_conflicts) {
    m_conflictsAgain = m_conflictsBusy;  // a build in progress reads the old list: redo after
    return;
  }
  m_conflictsBusy           = true;
  const int generation      = m_conflictsGeneration;
  auto index                = std::move(m_conflicts);  // the worker owns it meanwhile
  const std::string root    = utf8(qs((*m_session)->root()));
  const std::string profile = utf8(qs((*m_session)->profile_name()));
  QPointer<MainWindow> self(this);
  std::thread([self, generation, index, root, profile] {
    QString error;
    try {
      auto session = strata::open_session(root, profile);
      (*index)->update(*session);
    } catch (const std::exception& e) {
      error = QString::fromUtf8(e.what());
    }
    QMetaObject::invokeMethod(
        qApp,
        [self, generation, index, error] {
          if (!self || generation != self->m_conflictsGeneration) {
            return;
          }
          self->m_conflictsBusy = false;
          if (!error.isEmpty()) {
            self->showError(tr("cannot update the conflict index: %1").arg(error));
            self->buildConflicts();
            return;
          }
          self->m_conflicts = index;
          self->applyAllConflicts();
          self->applyCriteria();
          self->updateMarkers();
          if (self->m_conflictsAgain) {
            self->m_conflictsAgain = false;
            self->updateConflicts();
          }
        },
        Qt::QueuedConnection);
  }).detach();
}

void MainWindow::applyAllConflicts()
{
  if (!m_conflicts) {
    return;
  }
  m_updatingMods = true;
  for (auto it = m_modByName.cbegin(); it != m_modByName.cend(); ++it) {
    const QByteArray name = it.key().toUtf8();
    modlist::applyConflicts(it.value(), (*m_conflicts)->row(rust::Str(name.data(), std::size_t(name.size()))));
  }
  m_updatingMods = false;
}

void MainWindow::applyCriteria()
{
  if (!m_filterPanel->active()) {
    m_modProxy.setCriteriaPass(std::nullopt);
    ui->currentCategoryLabel->clear();
    updateModCounter();
    return;
  }
  // every row's facts, in one call (ModListSortProxy::filterMatchesMod's panel part)
  QStringList names;
  rust::Vec<strata::Facts> facts;
  std::vector<QStandardItem*> stack;
  for (int r = m_modModel->rowCount() - 1; r >= 0; --r) {
    stack.push_back(m_modModel->item(r));
  }
  while (!stack.empty()) {
    QStandardItem* item = stack.back();
    stack.pop_back();
    for (int c = item->rowCount() - 1; c >= 0; --c) {
      stack.push_back(item->child(c));
    }
    const QModelIndex index = item->index();
    const int kind          = item->data(modlist::KindRole).toInt();
    const auto icons = [&](int column) {
      return m_modModel->itemFromIndex(index.siblingAtColumn(column))->data(modlist::IconsRole).toStringList();
    };
    strata::Facts f;
    f.enabled        = item->checkState() == Qt::Checked;
    f.always_enabled = kind == modlist::Foreign || kind == modlist::Overwrite;
    f.separator      = kind == modlist::Separator;
    f.backup         = kind == modlist::Backup;
    f.foreign        = kind == modlist::Foreign;
    f.overwrite      = kind == modlist::Overwrite;
    for (int id : item->data(modlist::CategoriesRole).value<QList<int>>()) {
      f.categories.push_back(id);
    }
    f.conflicted   = !icons(modlist::ColConflictFlags).isEmpty();
    f.hidden_files = icons(modlist::ColFlags).contains(":/MO/gui/emblem_hidden_files");
    f.endorsed     = item->data(modlist::EndorsedRole).toBool();
    f.tracked      = item->data(modlist::TrackedRole).toBool();
    f.valid        = item->data(modlist::ValidRole).toBool();
    f.nexus_id     = item->data(modlist::NexusIdRole).toLongLong();
    f.update_available = item->data(modlist::UpdateRole).toInt() >= 2;
    for (int id : item->data(modlist::ContentIdsRole).value<QList<int>>()) {
      f.contents.push_back(std::uint8_t(id));
    }
    facts.push_back(std::move(f));
    names << item->data(modlist::InternalNameRole).toString();
  }
  const auto pass = strata::criteria_matches(m_filterPanel->criteria(), m_filterPanel->isAnd(),
                                             m_filterPanel->separators(), std::move(facts));
  QSet<QString> passing;
  for (qsizetype i = 0; i < names.size() && std::size_t(i) < pass.size(); ++i) {
    if (pass[std::size_t(i)]) {
      passing.insert(names[i]);
    }
  }
  m_modProxy.setCriteriaPass(std::move(passing));
  ui->currentCategoryLabel->setText(m_filterPanel->activeNames().join(", "));
  applyExpandState();
  updateModCounter();
}

void MainWindow::switchProfileForTest(const QString& name)
{
  const int index = ui->profileBox->findText(name);
  if (index > 0) {
    ui->profileBox->setCurrentIndex(index);
    switchProfile(index);
  }
}

void MainWindow::togglePluginForTest(const QString& name)
{
  m_pluginsTab->toggleForTest(name);
}

void MainWindow::tickFilterForTest(const QString& name)
{
  ui->categoriesGroup->setVisible(true);
  m_filterPanel->tick(name);
}

void MainWindow::updateMarkers()
{
  m_markers.clear();
  if (m_conflicts && m_session) {
    const auto colors = (*m_session)->marker_colors();
    auto color        = [](std::uint32_t c) {
      return QColor(int(c >> 24), int((c >> 16) & 0xff), int((c >> 8) & 0xff), int(c & 0xff));
    };
    // a selected collapsed separator stands for everything inside it (conflicts_from)
    QStringList names = selectedMods();
    if (m_rollUps.highlightFrom && ui->modList->selectionModel()) {
      for (const QModelIndex& index : ui->modList->selectionModel()->selectedRows(0)) {
        if (modlist::isCollapsedSeparator(ui->modList, index)) {
          modlist::forEachDescendant(index, [&names](const QModelIndex& child) {
            const QString name = child.data(modlist::InternalNameRole).toString();
            if (child.data(modlist::KindRole).toInt() != modlist::Overwrite && !names.contains(name)) {
              names << name;
            }
          });
        }
      }
    }
    const auto m = (*m_conflicts)->markers(rustNames(names));
    // weakest first, so the stronger colour wins a mod in several lists (markerColor's order)
    for (const auto& n : m.archive_overwritten) {
      m_markers[qs(n)] = color(colors.overwritten_archive);
    }
    for (const auto& n : m.archive_overwriting) {
      m_markers[qs(n)] = color(colors.overwriting_archive);
    }
    for (const auto& n : m.overwritten) {
      m_markers[qs(n)] = color(colors.overwritten_loose);
    }
    for (const auto& n : m.overwriting) {
      m_markers[qs(n)] = color(colors.overwriting_loose);
    }
  }
  ui->modList->viewport()->update();
}

void MainWindow::refreshDeploy()
{
  if (!m_session) {
    return;
  }
  if (m_deployRefreshRunning) {
    m_deployRefreshAgain = true;
    return;
  }
  m_deployRefreshRunning    = true;
  const std::string root    = utf8(qs((*m_session)->root()));
  const std::string profile = utf8(qs((*m_session)->profile_name()));
  QPointer<MainWindow> self(this);
  // on a worker with its own session: the daemon rebuilds its tree before it answers
  std::thread([self, root, profile] {
    QString line;
    bool failed = false;
    try {
      line = qs(strata::open_session(root, profile)->refresh_deploy());
    } catch (const std::exception& e) {
      line   = QString::fromUtf8(e.what());
      failed = true;
    }
    QMetaObject::invokeMethod(
        qApp,
        [self, line, failed] {
          if (!self) {
            return;
          }
          if (failed) {
            self->showError(tr("cannot refresh the live deploy: %1").arg(line));
          } else if (line != "not deployed") {
            qInfo("%s", qUtf8Printable(line));
          }
          self->m_deployRefreshRunning = false;
          if (self->m_deployRefreshAgain) {
            self->m_deployRefreshAgain = false;
            self->refreshDeploy();
          }
        },
        Qt::QueuedConnection);
  }).detach();
}

void MainWindow::updateModCounter()
{
  ui->activeModsCounter->display(modlist::visibleActiveCount(m_modProxy));
}

void MainWindow::loadExecutables()
{
  const int previous = ui->executablesListBox->currentIndex();
  QSignalBlocker block(ui->executablesListBox);
  ui->executablesListBox->clear();
  ui->executablesListBox->addItem(tr("<Edit...>"));  // MainWindow::refreshExecutablesList
  ui->menuRun->clear();
  // MainWindow::updateToolbar: the executables shown on the toolbar, before its separator
  for (QAction* a : ui->toolBar->actions()) {
    if (a->objectName().startsWith(QLatin1String("custom__"))) {
      ui->toolBar->removeAction(a);
      a->deleteLater();
    }
  }
  QAction* linksBefore = nullptr;
  for (QAction* a : ui->toolBar->actions()) {
    if (a->isSeparator()) {
      linksBefore = a;
    }
  }
  for (const auto& e : (*m_session)->executables()) {
    const QString title = qs(e.title);
    if (e.toolbar) {
      auto* link = new QAction(QIcon(QStringLiteral(":/MO/gui/executable")), title, ui->toolBar);
      link->setObjectName(QStringLiteral("custom__") + title);
      link->setStatusTip(qs(e.binary));
      connect(link, &QAction::triggered, this, [this, title] { runExecutable(title); });
      ui->toolBar->insertAction(linksBefore, link);
    }
    if (e.hide) {
      continue;  // "Hide in user interface"
    }
    ui->executablesListBox->addItem(QIcon(QStringLiteral(":/MO/gui/executable")), title);
    ui->menuRun->addAction(title, this, [this, title] { runExecutable(title); });
  }
  if (previous > 0 && previous < ui->executablesListBox->count()) {
    ui->executablesListBox->setCurrentIndex(previous);
  } else if (ui->executablesListBox->count() > 1) {
    ui->executablesListBox->setCurrentIndex(1);
  }
  m_oldExecutableIndex = ui->executablesListBox->currentIndex();
}

void MainWindow::loadDownloads()
{
  if (!m_session) {
    return;
  }
  m_downloadModel.removeRows(0, m_downloadModel.rowCount());
  const bool showHidden = ui->showHiddenBox->isChecked();
  for (const auto& d : (*m_session)->downloads()) {
    if (d.hidden && !showHidden) {
      continue;
    }
    // the install area's DownloadMeta gives the name as plain text already
    auto* name = cell(qs(d.display_name));
    name->setToolTip(qs(d.file_name));
    m_downloadModel.appendRow(
        {name, cell(qs(d.status)),
         cell(QLocale().formattedDataSize(qint64(d.size)), qint64(d.size)),
         cell(dateText(d.file_time), qint64(d.file_time)), cell(qs(d.mod_name)),
         cell(qs(d.version)), cell(qs(d.nexus_id)), cell(qs(d.game))});
  }
}

strata::DownloadContext MainWindow::downloadContext()
{
  strata::DownloadContext ctx{
      [this] { return m_session ? qs((*m_session)->root()) : QString(); },
      [this] { return m_session ? qs((*m_session)->profile_name()) : QString(); },
      [this] { return m_session ? qs((*m_session)->folders().downloads) : QString(); },
      [](const QString& line) { qInfo("%s", qUtf8Printable(line)); }, [this] { refresh(); }, {}};
  // install area: "Configure Mod?" after installing a mod with INI Tweaks
  ctx.configureIni = [this](const QString& mod) {
    openModInfo(mod, strata::conflicts::ModInfoDialog::IniFiles);
  };
  return ctx;
}

void MainWindow::addDownloadLink(const QString& url)
{
  if (m_downloadQueue) {
    m_downloadQueue->addLink(url);
  }
}

void MainWindow::openOverwrite()
{
  if (!m_session) {
    return;
  }
  OverwriteDialog dialog(qs((*m_session)->root()), qs((*m_session)->profile_name()),
                         qs((*m_session)->folders().overwrite), this);
  {
    GeometrySaver geometry(qs((*m_session)->root()), &dialog);
    dialog.exec();
  }
  if (dialog.changed()) {
    refresh();
  }
}

void MainWindow::openModInfo(const QString& mod, int tab)
{
  if (m_session && strata::conflicts::ModInfoDialog::open(
                       this, qs((*m_session)->root()), qs((*m_session)->profile_name()), mod, tab,
                       [this](const QString& n) { selectMod(n); },
                       [this](const QString& n, int step) { return neighbourMod(n, step); })) {
    refresh();
  }
}

QString MainWindow::neighbourMod(const QString& name, int step) const
{
  // ModListView::nextMod / prevMod: the next row of the view (into separators whether open
  // or not, wrapping at the end), skipping Overwrite, backups and separators
  auto* item = m_modByName.value(name);
  if (!item) {
    return {};
  }
  const QAbstractItemModel* model = &m_modProxy;
  auto next = [model](const QModelIndex& index) -> QModelIndex {  // ModListView::nextIndex
    if (model->rowCount(index) > 0) {
      return model->index(0, 0, index);
    }
    QModelIndex at = index;
    while (at.parent().isValid() && at.row() + 1 >= model->rowCount(at.parent())) {
      at = at.parent();
    }
    const int count = model->rowCount(at.parent());
    return count > 0 ? model->index((at.row() + 1) % count, 0, at.parent()) : QModelIndex();
  };
  auto prev = [model](const QModelIndex& index) -> QModelIndex {  // ModListView::prevIndex
    if (index.row() == 0 && index.parent().isValid()) {
      return index.parent();
    }
    if (index.row() == 0) {
      return {};  // the top of the list: no wrap here (C++'s -1 % n is -1 in MO2)
    }
    QModelIndex at = model->index(index.row() - 1, 0, index.parent());
    // the last row inside it, at any depth (MO2 looks one level deep: Leet's nesting is deeper)
    while (model->rowCount(at) > 0) {
      at = model->index(model->rowCount(at) - 1, 0, at);
    }
    return at;
  };
  const QModelIndex start = m_modProxy.mapFromSource(item->index());
  if (!start.isValid()) {
    return {};
  }
  QModelIndex index = start;
  for (;;) {
    index = step < 0 ? prev(index) : next(index);
    if (!index.isValid() || index == start) {
      return {};
    }
    const int kind = index.data(modlist::KindRole).toInt();
    if (kind == modlist::Regular) {
      return index.data(modlist::InternalNameRole).toString();
    }
  }
}

void MainWindow::previewDataFile(const QString& path)
{
  if (m_session) {
    strata::conflicts::previewMergedFile(this, qs((*m_session)->root()), qs((*m_session)->profile_name()),
                                         path, false);
  }
}


void MainWindow::runExecutable(const QString& title, const QStringList& extra)
{
  if (!m_session || (m_run && m_run->state() != QProcess::NotRunning)) {
    return;
  }
  // The same code as `strata run`, in its own process: the window never waits for the
  // mount or the game, and a crash there cannot take the window down.
  QStringList args{"run"};
  if (!title.isEmpty()) {
    args << title;
  }
  args << extra << "-i" << qs((*m_session)->root()) << "-p" << qs((*m_session)->profile_name());
  if (qEnvironmentVariableIsSet("STRATA_SHELL_RUN_DRY")) {
    args << "--dry-run";  // test hook: print the plan, launch nothing
  }
  if (!m_run) {
    m_run = new QProcess(this);
    auto forward = [](const QByteArray& data, bool error) {
      for (const QByteArray& line : data.split('\n')) {
        if (line.trimmed().isEmpty()) {
          continue;
        }
        if (error) {
          qWarning("%s", line.constData());
        } else {
          qInfo("%s", line.constData());
        }
      }
    };
    connect(m_run, &QProcess::readyReadStandardOutput, this, [this, forward] {
      while (m_run->canReadLine()) {
        const QByteArray line = m_run->readLine();
        forward(line, false);
      }
    });
    connect(m_run, &QProcess::readyReadStandardError, this,
            [this, forward] { forward(m_run->readAllStandardError(), true); });
    connect(m_run, &QProcess::finished, this, [this](int code, QProcess::ExitStatus status) {
      ui->startButton->setEnabled(true);
      if (status == QProcess::CrashExit || code != 0) {
        showError(tr("the run ended with exit code %1").arg(code));
      } else {
        m_statusText->setText(tr("the run ended"));
        qInfo("run finished");
      }
    });
    connect(m_run, &QProcess::errorOccurred, this, [this](QProcess::ProcessError) {
      ui->startButton->setEnabled(true);
      showError(tr("cannot start strata run: %1").arg(m_run->errorString()));
    });
  }
  ui->startButton->setEnabled(false);
  QString shown = title;
  m_statusText->setText(tr("running %1...").arg(shown));
  qInfo("strata %s", qUtf8Printable(args.join(' ')));
  // test hook: a fake `strata` that prints what `strata run` prints
  const QString program = qEnvironmentVariable("STRATA_SHELL_RUN_PROGRAM", QCoreApplication::applicationFilePath());
  m_run->start(program, args);
}

void MainWindow::openFolder(const QString& path)
{
  QDesktopServices::openUrl(QUrl::fromLocalFile(path));
}

void MainWindow::closeEvent(QCloseEvent* event)
{
  m_layout->flush();
  QMainWindow::closeEvent(event);
}

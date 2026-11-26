#include "conflicts/modinfodialog.h"
#include "geometrysaver.h"

#include "conflicts/preview.h"
#include "ui_modinfodialog.h"

#include "lcdnumber.h"

#include <QDesktopServices>

#include <algorithm>
#include <QDir>
#include <QDirIterator>
#include <QFileInfo>
#include <QLabel>
#include <QTabBar>
#include <QTreeWidgetItemIterator>
#include <QFileSystemModel>
#include <QHeaderView>
#include <QInputDialog>
#include <QListWidget>
#include <QMenu>
#include <QMessageBox>
#include <QShortcut>
#include <QSortFilterProxyModel>
#include <QStandardItemModel>
#include <QUrl>
#include <QVBoxLayout>

namespace strata::conflicts
{

namespace
{

QString qs(const ::rust::String& s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

rust::String rs(const QString& s)
{
  const QByteArray utf8 = s.toUtf8();
  return ::rust::String(utf8.data(), std::size_t(utf8.size()));
}

QStringList strings(const ::rust::Vec<::rust::String>& v)
{
  QStringList out;
  for (const auto& s : v) {
    out << qs(s);
  }
  return out;
}

constexpr int RealPathRole = Qt::UserRole + 1;  // the file on disk behind a conflicts row
constexpr int CategoryRole = Qt::UserRole + 2;
constexpr int ModsRole     = Qt::UserRole + 3;  // QStringList: the other mods of a conflicts row

QStringList mods(const ::rust::Vec<::rust::String>& v)
{
  QStringList out;
  for (const auto& s : v) {
    out << QString::fromUtf8(s.data(), qsizetype(s.size()));
  }
  return out;
}

// A proxy filtering on a line edit, any case, as MO2's FilterWidget does for each list.
QSortFilterProxyModel* filtered(QAbstractItemModel* source, QLineEdit* edit, QObject* parent,
                                int column = 0)
{
  auto* proxy = new QSortFilterProxyModel(parent);
  proxy->setSourceModel(source);
  proxy->setFilterCaseSensitivity(Qt::CaseInsensitive);
  proxy->setFilterKeyColumn(column);
  QObject::connect(edit, &QLineEdit::textChanged, proxy,
                   [proxy](const QString& text) { proxy->setFilterFixedString(text); });
  return proxy;
}

void openPath(const QString& path)
{
  QDesktopServices::openUrl(QUrl::fromLocalFile(path));
}

}  // namespace

ModInfoDialog::ModInfoDialog(const QString& instanceRoot, const QString& profile, const QString& mod,
                             QWidget* parent)
    : QDialog(parent), ui(new Ui::ModInfoDialog), m_mod(mod)
{
  m_view.emplace(open_mod_info(rs(instanceRoot), rs(profile), rs(mod)));
  m_dir = qs((*m_view)->dir());

  ui->setupUi(this);
  setWindowTitle(mod);
  // MO2 restores the size the user left; the form's own (735 x 500) is cramped for the lists.
  resize(std::max(width(), 1000), std::max(height(), 720));
  // Previous / Next: shown by open() when the main window gives the list's order
  ui->previousMod->hide();
  ui->nextMod->hide();
  connect(ui->previousMod, &QPushButton::clicked, this, [this] {
    m_step = -1;
    accept();
  });
  connect(ui->nextMod, &QPushButton::clicked, this, [this] {
    m_step = 1;
    accept();
  });
  connect(ui->close, &QPushButton::clicked, this, &QDialog::accept);

  setupFilesTab(m_text, ui->textFileList, ui->textFileFilter, ui->textFileEditor,
                strings((*m_view)->text_files()));
  setupFilesTab(m_ini, ui->iniFileList, ui->iniFileFilter, ui->iniFileEditor,
                strings((*m_view)->ini_files()));
  setupImages();
  setupPlugins();
  setupConflicts();
  setupCategories();
  setupNexus();
  setupNotes();
  setupFiletree();
  setTabColors();
}

ModInfoDialog::~ModInfoDialog()
{
  stopThumbnails();
}

void ModInfoDialog::stopThumbnails()
{
  if (m_thumbCancel) {
    m_thumbCancel->store(true);
  }
  if (m_thumbWorker) {
    m_thumbWorker->wait();
  }
}

void ModInfoDialog::loadThumbnails(QListWidget* list, const QStringList& paths)
{
  if (paths.isEmpty()) {
    return;
  }
  auto cancel   = std::make_shared<std::atomic<bool>>(false);
  m_thumbCancel = cancel;
  QPointer<QListWidget> target(list);
  m_thumbWorker = QThread::create([paths, cancel, target] {
    for (int row = 0; row < paths.size() && !cancel->load(); ++row) {
      QImage image = loadThumbnail(paths[row], 64);
      if (image.isNull()) {
        continue;
      }
      // the item is set on the UI thread; a cancelled run (refill, close) sets nothing
      QMetaObject::invokeMethod(target, [target, cancel, row, image = std::move(image)] {
        if (!cancel->load() && target && row < target->count()) {
          target->item(row)->setIcon(QIcon(QPixmap::fromImage(image)));
        }
      });
    }
  });
  m_thumbWorker->setParent(this);
  connect(m_thumbWorker, &QThread::finished, m_thumbWorker, &QObject::deleteLater);
  m_thumbWorker->start(QThread::LowPriority);
}

void ModInfoDialog::selectTab(int tab)
{
  if (tab >= 0 && tab < ui->tabWidget->count()) {
    ui->tabWidget->setCurrentIndex(tab);
  }
}

bool ModInfoDialog::open(QWidget* parent, const QString& instanceRoot, const QString& profile,
                         const QString& mod, int tab, std::function<void(const QString&)> goToMod,
                         std::function<QString(const QString&, int)> neighbour)
{
  bool changed = false;
  QString current = mod;
  try {
    // Previous / Next close the dialog (it saves as on Close) and open the other mod's on
    // the same tab (and place: the geometry is stored and restored): MO2's setMod refills
    // one dialog, the result is the same
    for (;;) {
      ModInfoDialog dialog(instanceRoot, profile, current, parent);
      dialog.m_goToMod = goToMod;
      if (neighbour) {
        dialog.ui->previousMod->show();
        dialog.ui->nextMod->show();
        // MO2's shortcuts: QKeySequence::MoveToPreviousPage / MoveToNextPage
        auto* prev = new QShortcut(QKeySequence::MoveToPreviousPage, &dialog);
        connect(prev, &QShortcut::activated, dialog.ui->previousMod, &QPushButton::click);
        auto* next = new QShortcut(QKeySequence::MoveToNextPage, &dialog);
        connect(next, &QShortcut::activated, dialog.ui->nextMod, &QPushButton::click);
      }
      dialog.selectTab(tab);
      {
        GeometrySaver saver(instanceRoot, &dialog);  // MO2's GeometrySaver in exec()
        dialog.exec();
      }
      changed = changed || dialog.changed();
      if (!dialog.m_goTo.isEmpty() && goToMod) {
        goToMod(dialog.m_goTo);
        break;
      }
      if (dialog.m_step == 0 || !neighbour) {
        break;
      }
      const QString other = neighbour(current, dialog.m_step);
      if (other.isEmpty() || other == current) {
        break;
      }
      current = other;
      tab     = dialog.ui->tabWidget->currentIndex();
    }
  } catch (const std::exception& e) {
    QMessageBox::warning(parent, tr("Mod information"), QString::fromUtf8(e.what()));
  }
  return changed;
}

// --- text and ini files (GenericFilesTab) ---------------------------------------------------

void ModInfoDialog::setupFilesTab(FilesTab& tab, QListView* list, QLineEdit* filterEdit,
                                  QPlainTextEdit* editor, const QStringList& files)
{
  tab.list   = list;
  tab.editor = editor;
  tab.model.setStringList(files);
  tab.filter = filtered(&tab.model, filterEdit, this);
  list->setModel(tab.filter);
  editor->setEnabled(false);
  connect(list->selectionModel(), &QItemSelectionModel::currentChanged, this,
          [this, &tab](const QModelIndex& current, const QModelIndex& previous) {
            if (current == previous) {
              return;
            }
            if (!saveIfModified(tab)) {
              QSignalBlocker block(tab.list->selectionModel());
              tab.list->setCurrentIndex(previous);
              return;
            }
            tab.current = current.data().toString();
            tab.editor->setEnabled(!tab.current.isEmpty());
            try {
              tab.editor->setPlainText(
                  tab.current.isEmpty() ? QString()
                                        : qs(read_text(rs(m_dir + "/" + tab.current))));
            } catch (const std::exception& e) {
              tab.editor->setPlainText(QString::fromUtf8(e.what()));
              tab.editor->setEnabled(false);
            }
            tab.editor->document()->setModified(false);
          });
}

bool ModInfoDialog::saveIfModified(FilesTab& tab)
{
  if (tab.current.isEmpty() || !tab.editor->document()->isModified()) {
    return true;
  }
  const auto answer = QMessageBox::question(
      this, tr("Save changes?"), tr("Save changes to \"%1\"?").arg(tab.current),
      QMessageBox::Yes | QMessageBox::No | QMessageBox::Cancel);
  if (answer == QMessageBox::Cancel) {
    return false;
  }
  if (answer == QMessageBox::Yes) {
    try {
      const QByteArray bytes = tab.editor->toPlainText().toUtf8();
      save_mod_file(rs(m_dir), rs(tab.current),
                    ::rust::Slice<const std::uint8_t>(
                        reinterpret_cast<const std::uint8_t*>(bytes.data()), std::size_t(bytes.size())));
      m_changed = true;
    } catch (const std::exception& e) {
      QMessageBox::warning(this, tr("Save failed"), QString::fromUtf8(e.what()));
      return false;
    }
  }
  tab.editor->document()->setModified(false);
  return true;
}

// --- images ----------------------------------------------------------------------------------

void ModInfoDialog::setupImages()
{
  // MO2 paints its own thumbnail strip; a list in icon mode does the same job here.
  auto* list = new QListWidget(ui->imagesThumbnails);
  list->setViewMode(QListView::ListMode);
  list->setIconSize(QSize(64, 64));
  list->setUniformItemSizes(true);
  auto* layout = new QVBoxLayout(ui->imagesThumbnails);
  layout->setContentsMargins(0, 0, 0, 0);
  layout->addWidget(list);
  ui->imagesScrollerVBar->hide();

  auto* picture = new QLabel(ui->imagesImage);
  picture->setAlignment(Qt::AlignCenter);
  auto* pictureLayout = new QVBoxLayout(ui->imagesImage);
  pictureLayout->setContentsMargins(0, 0, 0, 0);
  pictureLayout->addWidget(picture);

  const QStringList images = strings((*m_view)->images());
  auto refill = [this, list, images] {
    stopThumbnails();
    list->clear();
    QStringList paths;
    const QString filter = ui->imagesFilter->text();
    for (const QString& rel : images) {
      if (!ui->imagesShowDDS->isChecked() && rel.endsWith(".dds", Qt::CaseInsensitive)) {
        continue;
      }
      if (!filter.isEmpty() && !rel.contains(filter, Qt::CaseInsensitive)) {
        continue;
      }
      list->addItem(new QListWidgetItem(rel));
      paths << m_dir + "/" + rel;
    }
    // MO2 loads only the thumbnails it paints; decoding every texture here at once froze the
    // window on big texture mods (8,795 files, 33 GB)
    loadThumbnails(list, paths);
  };
  ui->imagesShowDDS->setChecked(true);
  connect(ui->imagesShowDDS, &QCheckBox::toggled, this, refill);
  connect(ui->imagesFilter, &QLineEdit::textChanged, this, refill);
  connect(list, &QListWidget::currentTextChanged, this, [this, picture](const QString& rel) {
    if (rel.isEmpty()) {
      picture->clear();
      ui->imagesPath->clear();
      ui->imagesSize->clear();
      return;
    }
    const QString path = m_dir + "/" + rel;
    QString error;
    const QImage image = loadImage(path, &error);
    ui->imagesPath->setText(path);
    if (image.isNull()) {
      picture->setText(error);
      ui->imagesSize->clear();
      return;
    }
    ui->imagesSize->setText(QString("%1 x %2").arg(image.width()).arg(image.height()));
    const QSize room = ui->imagesImage->size();
    picture->setPixmap(QPixmap::fromImage(
        image.width() > room.width() || image.height() > room.height()
            ? image.scaled(room, Qt::KeepAspectRatio, Qt::SmoothTransformation)
            : image));
  });
  connect(ui->previewPluginButton, &QToolButton::clicked, this, [this] {
    if (!ui->imagesPath->text().isEmpty()) {
      previewFile(this, ui->imagesPath->text(), m_mod);
    }
  });
  connect(ui->imagesExplore, &QToolButton::clicked, this, [this] {
    if (!ui->imagesPath->text().isEmpty()) {
      openPath(QFileInfo(ui->imagesPath->text()).absolutePath());
    }
  });
  refill();
  if (list->count() > 0) {
    list->setCurrentRow(0);
  }
}

// --- optional plugins (ESPsTab) --------------------------------------------------------------

void ModInfoDialog::setupPlugins()
{
  ui->activeESPList->setModel(&m_activePlugins);
  ui->inactiveESPList->setModel(&m_inactivePlugins);
  reloadPlugins();
  connect(ui->activateESP, &QToolButton::clicked, this, [this] {
    const QString rel = ui->inactiveESPList->currentIndex().data().toString();
    if (rel.isEmpty()) {
      return;
    }
    QString name = QFileInfo(rel).fileName();
    while (QFileInfo::exists(m_dir + "/" + name)) {
      bool ok = false;
      name = QInputDialog::getText(this, tr("File Exists"),
                                   tr("A file with that name exists, please enter a new one"),
                                   QLineEdit::Normal, QFileInfo(rel).fileName(), &ok);
      if (!ok) {
        return;
      }
      if (name.isEmpty()) {
        name = QFileInfo(rel).fileName();
      }
    }
    try {
      const QString now = qs(activate_plugin(rs(m_dir), rs(rel), rs(name)));
      m_cameFrom.insert(now, rel);
      m_changed = true;
    } catch (const std::exception& e) {
      QMessageBox::warning(this, tr("Failed to move file"), QString::fromUtf8(e.what()));
    }
    reloadPlugins();
  });
  connect(ui->deactivateESP, &QToolButton::clicked, this, [this] {
    const QString rel = ui->activeESPList->currentIndex().data().toString();
    if (rel.isEmpty()) {
      return;
    }
    try {
      deactivate_plugin(rs(m_dir), rs(rel), rs(m_cameFrom.take(rel)));
      m_changed = true;
    } catch (const std::exception& e) {
      QMessageBox::warning(this, tr("Failed to move file"), QString::fromUtf8(e.what()));
    }
    reloadPlugins();
  });
}

void ModInfoDialog::reloadPlugins()
{
  QStringList active, inactive;
  // At opening from the core's view; after a move the folder is listed again.
  if (m_activePlugins.rowCount() + m_inactivePlugins.rowCount() == 0 && m_cameFrom.isEmpty()) {
    for (const auto& p : (*m_view)->plugins()) {
      (p.active ? active : inactive) << qs(p.path);
    }
    m_activePlugins.setStringList(active);
    m_inactivePlugins.setStringList(inactive);
    return;
  }
  QDir dir(m_dir);
  QDirIterator it(m_dir, {"*.esp", "*.esm", "*.esl"}, QDir::Files, QDirIterator::Subdirectories);
  while (it.hasNext()) {
    const QString rel = dir.relativeFilePath(it.next());
    (rel.contains('/') ? inactive : active) << rel;
  }
  active.sort(Qt::CaseInsensitive);
  inactive.sort(Qt::CaseInsensitive);
  m_activePlugins.setStringList(active);
  m_inactivePlugins.setStringList(inactive);
}

// --- conflicts (GeneralConflictsTab, AdvancedConflictsTab) ------------------------------------

void ModInfoDialog::setupConflicts()
{
  struct General
  {
    QTreeView* tree;
    QLineEdit* filter;
    LCDNumber* count;
    QToolButton* expander;
    ::rust::Vec<GeneralRow> rows;
    QStringList headers;
  };
  General lists[] = {
      {ui->overwriteTree, ui->overwriteLineEdit, ui->overwriteCount, ui->overwriteExpander,
       (*m_view)->overwrites(), {tr("File"), tr("Overwritten Mods")}},
      {ui->overwrittenTree, ui->overwrittenLineEdit, ui->overwrittenCount, ui->overwrittenExpander,
       (*m_view)->overwritten(), {tr("File"), tr("Providing Mod")}},
      {ui->noConflictTree, ui->noConflictLineEdit, ui->noConflictCount, ui->noConflictExpander,
       (*m_view)->no_conflict(), {tr("File")}},
  };
  for (auto& g : lists) {
    auto* model = new QStandardItemModel(this);
    model->setHorizontalHeaderLabels(g.headers);
    for (const auto& r : g.rows) {
      QList<QStandardItem*> row{new QStandardItem(qs(r.shown))};
      row[0]->setData(qs(r.real), RealPathRole);
      row[0]->setData(mods(r.mods), ModsRole);
      if (g.headers.size() > 1) {
        row << new QStandardItem(qs(r.others));
      }
      for (auto* item : row) {
        item->setEditable(false);
      }
      model->appendRow(row);
    }
    g.tree->setModel(filtered(model, g.filter, this));
    g.tree->setSortingEnabled(true);
    g.tree->sortByColumn(0, Qt::AscendingOrder);
    g.count->display(int(g.rows.size()));
    g.tree->header()->setStretchLastSection(true);
    g.tree->resizeColumnToContents(0);
    g.tree->setContextMenuPolicy(Qt::CustomContextMenu);
    QTreeView* tree = g.tree;
    connect(tree, &QTreeView::customContextMenuRequested, this,
            [this, tree](const QPoint& pos) { conflictMenu(tree, pos, 0); });
    connect(tree, &QTreeView::doubleClicked, this, [this](const QModelIndex& index) {
      const QString real = index.siblingAtColumn(0).data(RealPathRole).toString();
      if (!real.isEmpty() && canPreview(real)) {
        previewFile(this, real, m_mod);
      }
    });
    // MO2's expanders fold each list away.
    g.expander->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
    g.expander->setCheckable(true);
    g.expander->setChecked(true);
    g.expander->setArrowType(Qt::DownArrow);
    QToolButton* expander = g.expander;
    connect(expander, &QToolButton::toggled, this, [tree, expander](bool open) {
      tree->setVisible(open);
      expander->setArrowType(open ? Qt::DownArrow : Qt::RightArrow);
    });
  }

  m_advanced = new QStandardItemModel(this);
  ui->conflictsAdvancedList->setModel(filtered(m_advanced, ui->conflictsAdvancedFilter, this, 1));
  ui->conflictsAdvancedList->setSortingEnabled(true);
  ui->conflictsAdvancedList->setContextMenuPolicy(Qt::CustomContextMenu);
  connect(ui->conflictsAdvancedList, &QTreeView::customContextMenuRequested, this,
          [this](const QPoint& pos) { conflictMenu(ui->conflictsAdvancedList, pos, 1); });
  connect(ui->conflictsAdvancedShowNoConflict, &QCheckBox::toggled, this, &ModInfoDialog::fillAdvanced);
  connect(ui->conflictsAdvancedShowAll, &QRadioButton::toggled, this, &ModInfoDialog::fillAdvanced);
  fillAdvanced();
}

void ModInfoDialog::fillAdvanced()
{
  m_advanced->clear();
  m_advanced->setHorizontalHeaderLabels({tr("Overwrites"), tr("File"), tr("Overwritten By")});
  const bool all       = ui->conflictsAdvancedShowAll->isChecked();
  const bool noneToo   = ui->conflictsAdvancedShowNoConflict->isChecked();
  for (const auto& r : (*m_view)->advanced()) {
    const QString before = qs(all ? r.before_all : r.before_nearest);
    const QString after  = qs(all ? r.after_all : r.after_nearest);
    if (before.isEmpty() && after.isEmpty() && !noneToo) {
      continue;
    }
    QList<QStandardItem*> row{new QStandardItem(before), new QStandardItem(qs(r.shown)),
                              new QStandardItem(after)};
    row[1]->setData(qs(r.real), RealPathRole);
    row[1]->setData(mods(r.mods), ModsRole);
    for (auto* item : row) {
      item->setEditable(false);
    }
    m_advanced->appendRow(row);
  }
  ui->conflictsAdvancedList->sortByColumn(1, Qt::AscendingOrder);
  for (int c = 0; c < 2; ++c) {
    ui->conflictsAdvancedList->resizeColumnToContents(c);
  }
}

void ModInfoDialog::conflictMenu(QTreeView* tree, const QPoint& pos, int pathColumn)
{
  const QModelIndex index = tree->indexAt(pos);
  if (!index.isValid()) {
    return;
  }
  const QString real = index.siblingAtColumn(pathColumn).data(RealPathRole).toString();
  QMenu menu(this);
  QAction* preview = menu.addAction(tr("&Preview"));
  preview->setEnabled(!real.isEmpty() && canPreview(real));
  QAction* open = menu.addAction(tr("&Open"));
  open->setEnabled(!real.isEmpty());
  QAction* explore = menu.addAction(tr("Open in &Explorer"));
  explore->setEnabled(!real.isEmpty());
  // MO2's "Go to" submenu: one entry per other mod of the conflict.
  QHash<QAction*, QString> goTo;
  const QStringList others = index.siblingAtColumn(pathColumn).data(ModsRole).toStringList();
  if (m_goToMod && !others.isEmpty()) {
    QMenu* sub = menu.addMenu(tr("&Go to..."));
    for (const QString& other : others) {
      if (!other.startsWith('<')) {  // <game>, <overwrite> are not rows of the mod list
        goTo.insert(sub->addAction(other), other);
      }
    }
    sub->setEnabled(!goTo.isEmpty());
  }
  QAction* chosen = menu.exec(tree->viewport()->mapToGlobal(pos));
  if (goTo.contains(chosen)) {
    m_goTo = goTo.value(chosen);
    accept();
    return;
  }
  if (chosen == preview) {
    previewFile(this, real, m_mod);
  } else if (chosen == open) {
    openPath(real);
  } else if (chosen == explore) {
    openPath(QFileInfo(real).absolutePath());
  }
}

// --- categories, Nexus, notes, file tree --------------------------------------------------

void ModInfoDialog::setupCategories()
{
  ui->categories->clear();
  QHash<int, QTreeWidgetItem*> items;
  const auto list = (*m_view)->categories();
  // Parents first: categories.dat lists a child after its parent, but be safe and loop.
  for (bool added = true; added;) {
    added = false;
    for (const auto& c : list) {
      if (items.contains(c.id) || (c.parent != 0 && !items.contains(c.parent))) {
        continue;
      }
      auto* item = c.parent == 0 ? new QTreeWidgetItem(ui->categories)
                                 : new QTreeWidgetItem(items.value(c.parent));
      item->setText(0, qs(c.name));
      item->setData(0, CategoryRole, c.id);
      item->setCheckState(0, c.checked ? Qt::Checked : Qt::Unchecked);
      items.insert(c.id, item);
      added = true;
    }
  }
  ui->categories->expandAll();

  auto refillPrimary = [this, items] {
    const int previous = ui->primaryCategories->currentData().toInt();
    ui->primaryCategories->clear();
    for (auto* item : items) {
      if (item->checkState(0) == Qt::Checked) {
        ui->primaryCategories->addItem(item->text(0), item->data(0, CategoryRole));
      }
    }
    const int at = ui->primaryCategories->findData(previous);
    ui->primaryCategories->setCurrentIndex(at >= 0 ? at : 0);
  };
  refillPrimary();
  const int at = ui->primaryCategories->findData((*m_view)->primary_category());
  if (at >= 0) {
    ui->primaryCategories->setCurrentIndex(at);
  }
  connect(ui->categories, &QTreeWidget::itemChanged, this, refillPrimary);
}

void ModInfoDialog::setupNexus()
{
  const auto n = (*m_view)->nexus();
  ui->modID->setText(n.mod_id > 0 ? QString::number(n.mod_id) : QString());
  ui->sourceGame->addItem(qs(n.game_name).isEmpty() ? tr("(managed game)") : qs(n.game_name));
  ui->version->setText(qs(n.version));
  ui->browser->setHtml(qs(n.description));
  ui->hasCustomURL->setChecked(n.custom_url);
  ui->customURL->setText(qs(n.url));
  // Read only: Nexus access is the install area's (strata-nexus).
  for (QWidget* w : std::initializer_list<QWidget*>{ui->modID, ui->sourceGame, ui->version,
                                                   ui->category, ui->hasCustomURL, ui->customURL}) {
    w->setEnabled(false);
  }
  ui->refresh->setEnabled(false);
  ui->endorse->setEnabled(false);
  ui->track->setEnabled(false);
  ui->endorse->setText(qs(n.endorsed));
  ui->track->setText(n.tracked ? tr("Tracked") : tr("Untracked"));
  const QString url = n.mod_id > 0 ? QString("https://www.nexusmods.com/skyrimspecialedition/mods/%1").arg(n.mod_id)
                                   : QString();
  ui->visitNexus->setEnabled(!url.isEmpty());
  connect(ui->visitNexus, &QPushButton::clicked, this, [url] { QDesktopServices::openUrl(QUrl(url)); });
  ui->visitCustomURL->setEnabled(n.custom_url && !qs(n.url).isEmpty());
  const QString custom = qs(n.url);
  connect(ui->visitCustomURL, &QPushButton::clicked, this, [custom] { QDesktopServices::openUrl(QUrl(custom)); });
}

void ModInfoDialog::setupNotes()
{
  ui->comments->setText(qs((*m_view)->comments()));
  ui->notes->setHtml(qs((*m_view)->notes()));
  ui->notes->document()->setModified(false);
  // The mod's colour is set from the mod list's context menu (shell).
  ui->setColorButton->hide();
  ui->resetColorButton->hide();
}

void ModInfoDialog::setupFiletree()
{
  auto* model = new QFileSystemModel(this);
  model->setReadOnly(true);
  ui->filetree->setModel(model);
  ui->filetree->setRootIndex(model->setRootPath(m_dir));
  ui->filetree->header()->setSectionResizeMode(0, QHeaderView::Stretch);
  connect(ui->openInExplorer, &QToolButton::clicked, this, [this] { openPath(m_dir); });
  connect(ui->filetree, &QTreeView::doubleClicked, this, [this, model](const QModelIndex& index) {
    const QString path = model->filePath(index);
    if (model->isDir(index)) {
      return;
    }
    if (canPreview(path)) {
      previewFile(this, path, m_mod);
    } else {
      openPath(path);
    }
  });
}

// ModInfoDialog::setTabsColors: a tab with nothing to show gets the disabled text colour.
void ModInfoDialog::setTabColors()
{
  const bool separator = (*m_view)->is_separator();
  const bool hasData[] = {
      m_text.model.rowCount() > 0,
      m_ini.model.rowCount() > 0,
      !(*m_view)->images().empty(),
      m_activePlugins.rowCount() + m_inactivePlugins.rowCount() > 0,
      ui->overwriteCount->intValue() + ui->overwrittenCount->intValue() > 0,
      true,
      !separator,
      true,
      !separator,
  };
  const QColor disabled = palette().color(QPalette::Disabled, QPalette::WindowText);
  for (int i = 0; i < ui->tabWidget->count() && i < int(std::size(hasData)); ++i) {
    ui->tabWidget->tabBar()->setTabTextColor(i, hasData[i] ? QColor() : disabled);
  }
}

// --- closing ---------------------------------------------------------------------------------

void ModInfoDialog::save()
{
  try {
    if (save_mod_notes(rs(m_dir), rs(ui->comments->text()),
                       rs(ui->notes->document()->isModified() ? ui->notes->toHtml()
                                                              : qs((*m_view)->notes())))) {
      m_changed = true;
    }
    ::rust::Vec<std::int32_t> checked;
    for (QTreeWidgetItemIterator it(ui->categories); *it; ++it) {
      if ((*it)->checkState(0) == Qt::Checked) {
        checked.push_back((*it)->data(0, CategoryRole).toInt());
      }
    }
    if (save_mod_categories(rs(m_dir), ui->primaryCategories->currentData().toInt(), std::move(checked))) {
      m_changed = true;
    }
  } catch (const std::exception& e) {
    QMessageBox::warning(this, tr("Mod information"), QString::fromUtf8(e.what()));
  }
}

void ModInfoDialog::done(int result)
{
  if (!saveIfModified(m_text) || !saveIfModified(m_ini)) {
    return;
  }
  save();
  QDialog::done(result);
}

}  // namespace strata::conflicts

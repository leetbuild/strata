#include "executablesdialog.h"

#include "common.h"
#include "ui_editexecutablesdialog.h"

#include <QCompleter>
#include <QFileDialog>
#include <QFileInfo>
#include <QMenu>
#include <QMessageBox>
#include <QPushButton>

namespace strata::modlist
{

ExecutablesDialog::ExecutablesDialog(const QString& root, const QString& profile, QWidget* parent)
    : QDialog(parent), ui(new Ui::EditExecutablesDialog), m_root(root), m_profile(profile)
{
  ui->setupUi(this);
  ui->splitter->setSizes({200, 1});
  ui->splitter->setStretchFactor(0, 0);
  ui->splitter->setStretchFactor(1, 1);

  // usvfs's forced library loading does not exist under the overlay; the game plugin's own
  // executables (Reset) are not modelled
  ui->forceLoadLibraries->setVisible(false);
  ui->configureLibraries->setVisible(false);
  ui->reset->setEnabled(false);
  ui->reset->setToolTip(tr("Strata has no built-in executables to reset to."));

  try {
    QStringList mods;
    for (const auto& name : executable_mod_names(utf8(root), utf8(profile))) {
      mods.push_back(qs(name));
    }
    ui->mods->addItems(mods);
    auto* completer = new QCompleter(mods, this);
    completer->setCaseSensitivity(Qt::CaseInsensitive);
    completer->setCompletionMode(QCompleter::UnfilteredPopupCompletion);
    ui->mods->setCompleter(completer);

    for (const auto& e : executable_rows(utf8(root), utf8(profile))) {
      Row r;
      r.title            = qs(e.title);
      r.binary           = qs(e.binary);
      r.arguments        = qs(e.arguments);
      r.workingDirectory = qs(e.working_directory);
      r.steamAppID       = qs(e.steam_app_id);
      r.outputMod        = qs(e.output_mod);
      r.toolbar          = e.toolbar;
      r.ownIcon          = e.own_icon;
      r.hide             = e.hide;
      m_original.push_back(utf8(r.title));
      m_rows.push_back(r);
    }
  } catch (const std::exception& e) {
    QMessageBox::warning(this, windowTitle(), QString::fromUtf8(e.what()));
  }

  auto* menu = new QMenu(this);
  menu->addAction(tr("Add from file..."), this, [this] { addFromFile(); });
  menu->addAction(tr("Add empty"), this, [this] {
    Row row;
    row.title = tr("New Executable");
    addNew(row);
  });
  menu->addAction(tr("Clone selected"), this, [this] {
    if (current() >= 0) {
      addNew(m_rows[current()]);
    }
  });
  ui->add->setMenu(menu);

  connect(ui->list, &QListWidget::currentRowChanged, this, [this](int row) { showRow(row); });
  connect(ui->remove, &QToolButton::clicked, this, [this] { removeCurrent(); });
  connect(ui->up, &QToolButton::clicked, this, [this] { moveCurrent(-1); });
  connect(ui->down, &QToolButton::clicked, this, [this] { moveCurrent(+1); });
  connect(ui->browseBinary, &QPushButton::clicked, this, [this] { browseBinary(); });
  connect(ui->browseWorkingDirectory, &QPushButton::clicked, this, [this] { browseWorkingDirectory(); });
  connect(ui->title, &QLineEdit::textChanged, this, [this](const QString& t) { setTitle(t); });
  connect(ui->title, &QLineEdit::editingFinished, this, [this] {
    if (current() >= 0) {
      ui->title->setText(m_rows[current()].title);  // on_title_editingFinished
    }
  });
  for (auto* edit : {ui->binary, ui->workingDirectory, ui->arguments, ui->steamAppID}) {
    connect(edit, &QLineEdit::textChanged, this, [this] { saveRow(); });
  }
  connect(ui->mods, &QComboBox::currentTextChanged, this, [this] { saveRow(); });
  for (auto* box : {ui->useApplicationIcon, ui->hide}) {
    connect(box, &QCheckBox::toggled, this, [this] { saveRow(); });
  }
  connect(ui->overwriteSteamAppID, &QCheckBox::toggled, this, [this](bool on) {
    ui->steamAppID->setEnabled(on);
    saveRow();
  });
  connect(ui->createFilesInMod, &QCheckBox::toggled, this, [this](bool on) {
    ui->mods->setEnabled(on);
    saveRow();
  });
  connect(ui->buttons, &QDialogButtonBox::clicked, this, [this](QAbstractButton* b) {
    if (b == ui->buttons->button(QDialogButtonBox::Ok)) {
      if (commit()) {
        accept();
      }
    } else if (b == ui->buttons->button(QDialogButtonBox::Apply)) {
      commit();
    } else {
      reject();
    }
  });

  fillList(m_rows.empty() ? -1 : 0);
}

ExecutablesDialog::~ExecutablesDialog() = default;

int ExecutablesDialog::current() const
{
  const int row = ui->list->currentRow();
  return row >= 0 && row < int(m_rows.size()) ? row : -1;
}

void ExecutablesDialog::fillList(int select)
{
  {
    QSignalBlocker block(ui->list);
    ui->list->clear();
    for (const auto& r : m_rows) {
      ui->list->addItem(r.title);
    }
  }
  ui->list->setCurrentRow(select);
  showRow(select);
}

// updateUI / setEdits / clearEdits
void ExecutablesDialog::showRow(int index)
{
  m_settingUI   = true;
  const bool on = index >= 0 && index < int(m_rows.size());
  const Row r   = on ? m_rows[index] : Row{};
  ui->title->setText(r.title);
  ui->binary->setText(r.binary);
  ui->workingDirectory->setText(r.workingDirectory);
  ui->arguments->setText(r.arguments);
  ui->overwriteSteamAppID->setChecked(!r.steamAppID.isEmpty());
  ui->steamAppID->setText(r.steamAppID);
  ui->steamAppID->setEnabled(on && !r.steamAppID.isEmpty());
  ui->createFilesInMod->setChecked(!r.outputMod.isEmpty());
  ui->mods->setCurrentIndex(r.outputMod.isEmpty() ? -1 : ui->mods->findText(r.outputMod));
  ui->mods->setEnabled(on && !r.outputMod.isEmpty());
  ui->useApplicationIcon->setChecked(r.ownIcon);
  ui->hide->setChecked(r.hide);
  for (QWidget* w : std::initializer_list<QWidget*>{
           ui->title, ui->binary, ui->browseBinary, ui->workingDirectory, ui->browseWorkingDirectory,
           ui->arguments, ui->overwriteSteamAppID, ui->createFilesInMod, ui->useApplicationIcon, ui->hide}) {
    w->setEnabled(on);
  }
  ui->up->setEnabled(on && index > 0);
  ui->down->setEnabled(on && index + 1 < int(m_rows.size()));
  ui->remove->setEnabled(on);
  m_settingUI = false;
}

void ExecutablesDialog::saveRow()
{
  const int i = current();
  if (m_settingUI || i < 0) {
    return;
  }
  Row& r             = m_rows[i];
  r.binary           = ui->binary->text();
  r.workingDirectory = ui->workingDirectory->text();
  r.arguments        = ui->arguments->text();
  r.steamAppID       = ui->overwriteSteamAppID->isChecked() ? ui->steamAppID->text() : QString();
  r.outputMod        = ui->createFilesInMod->isChecked() ? ui->mods->currentText() : QString();
  r.ownIcon          = ui->useApplicationIcon->isChecked();
  r.hide             = ui->hide->isChecked();
}

// on_title_textChanged: empty titles and titles another entry has are not taken
void ExecutablesDialog::setTitle(const QString& text)
{
  const int i = current();
  if (m_settingUI || i < 0) {
    return;
  }
  const QString t = text.trimmed();
  if (t.isEmpty()) {
    return;
  }
  for (int j = 0; j < int(m_rows.size()); ++j) {
    if (j != i && m_rows[j].title == t) {
      return;
    }
  }
  m_rows[i].title = t;
  ui->list->item(i)->setText(t);
}

// addNew: a unique title, appended and selected
void ExecutablesDialog::addNew(Row row)
{
  rust::Vec<rust::String> titles;
  for (const auto& r : m_rows) {
    titles.push_back(rs(r.title));
  }
  // makeNonConflictingTitle through the core, treating the title as a "New Executable"
  const QString unique = qs(executable_title_for_binary(std::move(titles), "New Executable", utf8(row.title) + ".x"));
  if (!unique.isEmpty()) {
    row.title = unique;
  }
  m_rows.push_back(row);
  fillList(int(m_rows.size()) - 1);
}

void ExecutablesDialog::addFromFile()
{
  const QString file = QFileDialog::getOpenFileName(this, tr("Select a binary"), QString(),
                                                    tr("Executable (*.exe *.bat *.jar);;All files (*)"));
  if (file.isEmpty()) {
    return;
  }
  Row r;
  r.title  = QFileInfo(file).completeBaseName();
  r.binary = qs(executable_binary_for_storage(utf8(file)));
  addNew(r);
}

void ExecutablesDialog::removeCurrent()
{
  const int i = current();
  if (i < 0) {
    return;
  }
  m_rows.erase(m_rows.begin() + i);
  fillList(std::min(i, int(m_rows.size()) - 1));
}

void ExecutablesDialog::moveCurrent(int by)
{
  const int i = current();
  const int j = i + by;
  if (i < 0 || j < 0 || j >= int(m_rows.size())) {
    return;
  }
  std::swap(m_rows[i], m_rows[j]);
  fillList(j);
}

// on_browseBinary_clicked + setBinary: the title follows when it is a "New Executable"
void ExecutablesDialog::browseBinary()
{
  const int i = current();
  if (i < 0) {
    return;
  }
  const QString start = qs(executable_host_path(utf8(m_root), utf8(ui->binary->text())));
  const QString file  = QFileDialog::getOpenFileName(this, tr("Select a binary"), QFileInfo(start).absolutePath(),
                                                     tr("Executable (*.exe *.bat *.jar);;All files (*)"));
  if (file.isEmpty()) {
    return;
  }
  const QString stored = qs(executable_binary_for_storage(utf8(file)));
  ui->binary->setText(stored);
  rust::Vec<rust::String> titles;
  for (int j = 0; j < int(m_rows.size()); ++j) {
    if (j != i) {
      titles.push_back(rs(m_rows[j].title));
    }
  }
  const QString title = qs(executable_title_for_binary(std::move(titles), utf8(ui->title->text()), utf8(stored)));
  if (!title.isEmpty()) {
    ui->title->setText(title);
  }
}

void ExecutablesDialog::browseWorkingDirectory()
{
  const QString start = qs(executable_host_path(utf8(m_root), utf8(ui->workingDirectory->text())));
  const QString dir   = QFileDialog::getExistingDirectory(this, tr("Select a directory"), start);
  if (!dir.isEmpty()) {
    ui->workingDirectory->setText(qs(executable_binary_for_storage(utf8(dir))));
  }
}

// commitChanges
bool ExecutablesDialog::commit()
{
  rust::Vec<rust::String> original;
  for (const auto& t : m_original) {
    original.push_back(rust::String(t));
  }
  rust::Vec<ExecutableRow> rows;
  for (const auto& r : m_rows) {
    ExecutableRow e;
    e.title             = rs(r.title);
    e.binary            = rs(r.binary);
    e.arguments         = rs(r.arguments);
    e.working_directory = rs(r.workingDirectory);
    e.steam_app_id      = rs(r.steamAppID);
    e.toolbar           = r.toolbar;
    e.own_icon          = r.ownIcon;
    e.hide              = r.hide;
    e.output_mod        = rs(r.outputMod);
    rows.push_back(std::move(e));
  }
  try {
    m_changed |= executables_commit(utf8(m_root), utf8(m_profile), std::move(original), std::move(rows));
  } catch (const std::exception& e) {
    QMessageBox::warning(this, windowTitle(), QString::fromUtf8(e.what()));
    return false;
  }
  m_original.clear();
  for (const auto& r : m_rows) {
    m_original.push_back(utf8(r.title));
  }
  return true;
}

}  // namespace strata::modlist

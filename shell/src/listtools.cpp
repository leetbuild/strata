#include "listtools.h"

#include "strata_bridge/bridge.h"
#include "ui_savetextasdialog.h"
#include "ui_syncoverwritedialog.h"

#include <QApplication>
#include <QButtonGroup>
#include <QCheckBox>
#include <QClipboard>
#include <QComboBox>
#include <QDialogButtonBox>
#include <QFile>
#include <QFileDialog>
#include <QGridLayout>
#include <QGroupBox>
#include <QHeaderView>
#include <QLabel>
#include <QMessageBox>
#include <QRadioButton>
#include <QVBoxLayout>

namespace listtools
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

// SaveTextAsDialog: the text, Copy To Clipboard, Save As..., Close. The file gets the text
// as made (CRLF), not the editor's copy of it.
void saveTextAs(QWidget* parent, const QString& text)
{
  QDialog dlg(parent);
  Ui::SaveTextAsDialog ui;
  ui.setupUi(&dlg);
  dlg.setWindowTitle(QObject::tr("Export to csv"));
  ui.textEdit->setPlainText(text);
  QObject::connect(ui.closeBtn, &QPushButton::clicked, &dlg, &QDialog::close);
  QObject::connect(ui.clipboardBtn, &QPushButton::clicked, &dlg,
                   [&] { QApplication::clipboard()->setText(ui.textEdit->toPlainText()); });
  QObject::connect(ui.saveAsBtn, &QPushButton::clicked, &dlg, [&] {
    const QString fileName = QFileDialog::getSaveFileName(&dlg, QObject::tr("Save CSV"), QString(),
                                                          QObject::tr("Text Files") + " (*.txt *.csv)");
    if (fileName.isEmpty()) {
      return;
    }
    QFile file(fileName);
    if (!file.open(QIODevice::WriteOnly)) {
      QMessageBox::critical(&dlg, QObject::tr("Error"),
                            QObject::tr("failed to open \"%1\" for writing").arg(fileName));
      return;
    }
    file.write(text.toUtf8());
  });
  dlg.exec();
}

}  // namespace

void exportCsv(QWidget* parent, strata::Session& session, const QStringList& visible)
{
  QDialog selection(parent);
  selection.setObjectName("exportCsvDialog");
  auto* grid = new QGridLayout;
  selection.setWindowTitle(QObject::tr("Export to csv"));
  grid->addWidget(new QLabel(QObject::tr("CSV (Comma Separated Values) is a format that can be imported in programs "
                                         "like Excel to create a spreadsheet.\nYou can also use online editors and "
                                         "converters instead.")));
  auto* groupBoxRows = new QGroupBox(QObject::tr("Select what mods you want export:"));
  auto* all          = new QRadioButton(QObject::tr("All installed mods"));
  auto* active       = new QRadioButton(QObject::tr("Only active (checked) mods from your current profile"));
  auto* shown        = new QRadioButton(QObject::tr("All currently visible mods in the mod list"));
  auto* vbox         = new QVBoxLayout;
  vbox->addWidget(all);
  vbox->addWidget(active);
  vbox->addWidget(shown);
  vbox->addStretch(1);
  groupBoxRows->setLayout(vbox);
  grid->addWidget(groupBoxRows);
  auto* rows = new QButtonGroup(&selection);
  rows->addButton(all, 0);
  rows->addButton(active, 1);
  rows->addButton(shown, 2);
  all->setChecked(true);

  auto* groupBoxColumns = new QGroupBox(QObject::tr("Choose what Columns to export:"));
  groupBoxColumns->setFlat(true);
  // checkbox text, field, checked (MO2's order and defaults)
  const std::vector<std::tuple<QString, QString, bool>> columns{
      {QObject::tr("Mod_Priority"), "#Mod_Priority", true},
      {QObject::tr("Mod_Name"), "#Mod_Name", true},
      {QObject::tr("Mod_Status"), "#Mod_Status", true},
      {QObject::tr("Notes_column"), "#Note", false},
      {QObject::tr("Primary_Category"), "#Primary_Category", false},
      {QObject::tr("Nexus_ID"), "#Nexus_ID", false},
      {QObject::tr("Mod_Nexus_URL"), "#Mod_Nexus_URL", false},
      {QObject::tr("Mod_Version"), "#Mod_Version", false},
      {QObject::tr("Install_Date"), "#Install_Date", false},
      {QObject::tr("Download_File_Name"), "#Download_File_Name", false},
  };
  auto* vbox1 = new QVBoxLayout;
  std::vector<std::pair<QCheckBox*, QString>> boxes;
  for (const auto& [text, field, checked] : columns) {
    auto* box = new QCheckBox(text);
    box->setChecked(checked);
    vbox1->addWidget(box);
    boxes.emplace_back(box, field);
  }
  groupBoxColumns->setLayout(vbox1);
  grid->addWidget(groupBoxColumns);
  auto* buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
  QObject::connect(buttons, &QDialogButtonBox::accepted, &selection, &QDialog::accept);
  QObject::connect(buttons, &QDialogButtonBox::rejected, &selection, &QDialog::reject);
  grid->addWidget(buttons);
  selection.setLayout(grid);
  if (selection.exec() != QDialog::Accepted) {
    return;
  }
  rust::Vec<rust::String> names, fields;
  for (const QString& v : visible) {
    names.push_back(rs(v));
  }
  for (const auto& [box, field] : boxes) {
    if (box->isChecked()) {
      fields.push_back(rs(field));
    }
  }
  if (fields.empty()) {
    QMessageBox::critical(parent, QObject::tr("Error"), QObject::tr("export failed: %1").arg(QObject::tr("no fields set up yet!")));
    return;
  }
  const QString text = qs(session.export_csv(std::uint8_t(rows->checkedId()), std::move(names), std::move(fields)));
  saveTextAs(parent, text);
}

bool syncOverwrite(QWidget* parent, strata::Session& session)
{
  rust::Vec<strata::SyncFile> files;
  try {
    files = session.sync_overwrite_files();
  } catch (const std::exception& e) {
    QMessageBox::critical(parent, QObject::tr("Sync Overwrite"), QString::fromUtf8(e.what()));
    return false;
  }
  QDialog dlg(parent);
  Ui::SyncOverwriteDialog ui;
  ui.setupUi(&dlg);
  auto* rootItem = new QTreeWidgetItem(ui.syncTree, QStringList("<data>"));
  ui.syncTree->addTopLevelItem(rootItem);
  std::vector<std::pair<QString, QComboBox*>> leaves;
  for (const auto& f : files) {
    const QString path = qs(f.path);
    QTreeWidgetItem* parentItem = rootItem;
    const QStringList parts      = path.split('/');
    for (int i = 0; i < parts.size() - 1; ++i) {
      QTreeWidgetItem* found = nullptr;
      for (int c = 0; c < parentItem->childCount() && !found; ++c) {
        if (parentItem->child(c)->text(0) == parts[i] && parentItem->child(c)->childCount() > 0) {
          found = parentItem->child(c);
        }
      }
      if (!found) {
        found = new QTreeWidgetItem(parentItem, QStringList(parts[i]));
      }
      parentItem = found;
    }
    auto* item  = new QTreeWidgetItem(parentItem, QStringList(parts.last()));
    auto* combo = new QComboBox(ui.syncTree);
    combo->addItem(QObject::tr("<don't sync>"), QString());
    // MO2 lists the providers lowest first and picks the last: the one that would win
    for (std::size_t i = f.mods.size(); i-- > 0;) {
      combo->addItem(qs(f.mods[i]), qs(f.mods[i]));
    }
    combo->setCurrentIndex(combo->count() - 1);
    ui.syncTree->setItemWidget(item, 1, combo);
    leaves.emplace_back(path, combo);
  }
  ui.syncTree->expandAll();
  ui.syncTree->header()->setSectionResizeMode(0, QHeaderView::Stretch);
  ui.syncTree->header()->setSectionResizeMode(1, QHeaderView::Interactive);
  if (dlg.exec() != QDialog::Accepted) {
    return false;
  }
  rust::Vec<rust::String> paths, mods;
  for (const auto& [path, combo] : leaves) {
    const QString mod = combo->currentData().toString();
    if (!mod.isEmpty()) {
      paths.push_back(rs(path));
      mods.push_back(rs(mod));
    }
  }
  if (paths.empty()) {
    return false;
  }
  try {
    const auto errors = session.sync_overwrite(std::move(paths), std::move(mods));
    for (const auto& e : errors) {
      qCritical("%s", std::string(e).c_str());
    }
    if (!errors.empty()) {
      QMessageBox::warning(parent, QObject::tr("Sync Overwrite"),
                           QObject::tr("%n file(s) could not be moved (see the log).", nullptr, int(errors.size())));
    }
  } catch (const std::exception& e) {
    QMessageBox::critical(parent, QObject::tr("Sync Overwrite"), QString::fromUtf8(e.what()));
    return false;
  }
  return true;
}

}  // namespace listtools

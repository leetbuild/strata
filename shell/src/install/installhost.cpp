#include "installhost.h"

#include "fomoddialog.h"
#include "strata_bridge/bridge_install.h"
#include "ui_baincomplexinstallerdialog.h"
#include "ui_installdialog.h"
#include "ui_multiarchivedialog.h"
#include "ui_queryoverwritedialog.h"
#include "ui_simpleinstalldialog.h"

#include <QApplication>
#include <QCheckBox>
#include <QCompleter>
#include <QDialogButtonBox>
#include <QDir>
#include <QInputDialog>
#include <QJsonArray>
#include <QJsonDocument>
#include <QLabel>
#include <QLineEdit>
#include <QMenu>
#include <QMessageBox>
#include <QPlainTextEdit>
#include <QProgressDialog>
#include <QPushButton>
#include <QStyle>
#include <QThread>
#include <QVBoxLayout>

namespace strata
{

namespace
{

QString qs(rust::Str s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

void fillNames(QComboBox* combo, const QJsonObject& r)
{
  for (const auto& v : r["variants"].toArray()) {
    combo->addItem(v.toString());
  }
  combo->setCurrentText(r["name"].toString());
  combo->completer()->setCaseSensitivity(Qt::CaseSensitive);
}

QStringList strings(const QJsonValue& v)
{
  QStringList out;
  for (const auto& s : v.toArray()) {
    out << s.toString();
  }
  return out;
}

}  // namespace

InstallDialogs::InstallDialogs(QWidget* parent) : QObject(parent), m_parent(parent) {}

InstallDialogs::~InstallDialogs()
{
  delete m_fomod;
  delete m_progress;
}

rust::String InstallDialogs::ask(rust::Str kind, rust::Str request)
{
  const QString k = qs(kind);
  const QJsonObject r = QJsonDocument::fromJson(qs(request).toUtf8()).object();
  QJsonObject a;
  if (QThread::currentThread() == thread()) {
    a = show(k, r);
  } else {
    QMetaObject::invokeMethod(this, [&] { a = show(k, r); }, Qt::BlockingQueuedConnection);
  }
  return rust::String(QJsonDocument(a).toJson(QJsonDocument::Compact).toStdString());
}

bool InstallDialogs::progress(uint8_t percent)
{
  QMetaObject::invokeMethod(this, [this, percent] {
    if (!m_progress) {
      m_progress = new QProgressDialog(tr("Extracting files"), tr("Cancel"), 0, 100, m_parent);
      m_progress->setWindowModality(Qt::WindowModal);
      m_progress->setMinimumDuration(300);
      m_progress->setAutoReset(false);
      connect(m_progress, &QProgressDialog::canceled, this, [this] { m_cancelled = true; });
    }
    m_progress->setValue(percent);
    if (percent >= 100) {
      m_progress->hide();
    }
  });
  return !m_cancelled;
}

QJsonObject InstallDialogs::show(const QString& kind, const QJsonObject& r)
{
  if (kind == "quick") {
    return quick(r);
  } else if (kind == "bain") {
    return bain(r);
  } else if (kind == "bain_unsure") {
    return question(tr("May be BAIN installer"),
                    tr("This installer looks like it may contain a BAIN installer but I'm not sure. "
                       "Install as BAIN installer?"));
  } else if (kind == "bundle") {
    return bundle(r);
  } else if (kind == "fomod") {
    return fomod(r);
  } else if (kind == "fomod_missing") {
    QString text = tr("The following files or folders  were not found in the archive. This is likely due "
                      "to an incorrect FOMOD installer. This mod may not work properly.") +
                   "\n\n";
    for (const auto& m : strings(r["missing"])) {
      text += "- " + m + "\n";
    }
    QMessageBox box(QMessageBox::Warning, tr("Missing files or folders"), text, QMessageBox::NoButton, m_parent);
    auto* yes = box.addButton(tr("Install anyway"), QMessageBox::AcceptRole);
    box.addButton(tr("Cancel"), QMessageBox::RejectRole);
    box.exec();
    return {{"yes", box.clickedButton() == yes}};
  } else if (kind == "manual") {
    return manual(r);
  } else if (kind == "root_sort") {
    return rootSort(r);
  } else if (kind == "clash") {
    return clash(r);
  } else if (kind == "question") {
    // QuestionBoxMemory: Yes / No with "Remember selection"
    QMessageBox box(QMessageBox::Question, r["title"].toString(), r["text"].toString(),
                    QMessageBox::Yes | QMessageBox::No, m_parent);
    box.setDefaultButton(r["default"].toBool() ? QMessageBox::Yes : QMessageBox::No);
    auto* keep = new QCheckBox(tr("Remember selection"));
    box.setCheckBox(keep);
    const bool yes = box.exec() == QMessageBox::Yes;
    return {{"yes", yes}, {"remember", keep->isChecked()}};
  } else if (kind == "password") {
    // InstallationManager::queryPassword
    bool ok = false;
    const QString label = r["wrong"].toBool() ? tr("Wrong password, try again") : tr("Password");
    const QString pw =
        QInputDialog::getText(m_parent, tr("Password required"), label, QLineEdit::Password, QString(), &ok);
    return {{"action", ok ? "ok" : "cancel"}, {"password", pw}};
  } else if (kind == "ini_tweaks") {
    // OrganizerCore::doInstall
    return question(tr("Configure Mod"), tr("This mod contains ini tweaks. Do you want to configure them now?"));
  }
  return {{"action", "cancel"}};
}

QJsonObject InstallDialogs::question(const QString& title, const QString& text)
{
  const auto b = QMessageBox::question(m_parent, title, text, QMessageBox::Yes | QMessageBox::No);
  return {{"yes", b == QMessageBox::Yes}};
}

// installer_quick's SimpleInstallDialog
QJsonObject InstallDialogs::quick(const QJsonObject& r)
{
  QDialog dialog(m_parent);
  Ui::SimpleInstallDialog ui;
  ui.setupUi(&dialog);
  fillNames(ui.nameCombo, r);
  QString action = "cancel";
  QObject::connect(ui.okBtn, &QPushButton::clicked, &dialog, [&] { action = "ok"; dialog.accept(); });
  QObject::connect(ui.manualBtn, &QPushButton::clicked, &dialog, [&] { action = "manual"; dialog.accept(); });
  QObject::connect(ui.cancelBtn, &QPushButton::clicked, &dialog, &QDialog::reject);
  dialog.exec();
  return {{"action", action}, {"name", ui.nameCombo->currentText()}};
}

// installer_bain's BainComplexInstallerDialog (package.txt is not offered)
QJsonObject InstallDialogs::bain(const QJsonObject& r)
{
  QDialog dialog(m_parent);
  Ui::BainComplexInstallerDialog ui;
  ui.setupUi(&dialog);
  fillNames(ui.nameCombo, r);
  for (const auto& v : r["packages"].toArray()) {
    const auto p = v.toObject();
    auto* item = new QListWidgetItem(p["name"].toString(), ui.optionsList);
    item->setFlags(item->flags() | Qt::ItemIsUserCheckable);
    item->setCheckState(p["checked"].toBool() ? Qt::Checked : Qt::Unchecked);
  }
  ui.packageBtn->setEnabled(false);
  QString action = "cancel";
  QObject::connect(ui.okBtn, &QPushButton::clicked, &dialog, [&] { action = "ok"; dialog.accept(); });
  QObject::connect(ui.manualBtn, &QPushButton::clicked, &dialog, [&] { action = "manual"; dialog.accept(); });
  QObject::connect(ui.cancelBtn, &QPushButton::clicked, &dialog, &QDialog::reject);
  dialog.exec();
  QJsonArray selected;
  for (int i = 0; i < ui.optionsList->count(); ++i) {
    if (ui.optionsList->item(i)->checkState() == Qt::Checked) {
      selected.append(ui.optionsList->item(i)->text());
    }
  }
  return {{"action", action}, {"name", ui.nameCombo->currentText()}, {"selected", selected}};
}

// installer_bundle's MultiArchiveDialog: one button per archive
QJsonObject InstallDialogs::bundle(const QJsonObject& r)
{
  QDialog dialog(m_parent);
  Ui::MultiArchiveDialog ui;
  ui.setupUi(&dialog);
  QString action = "cancel", pick;
  for (const auto& a : strings(r["archives"])) {
    auto* button = new QPushButton(a, &dialog);
    // MO2 styles these buttons with a widget sheet; docs/DESIGN.md ("Style") allows the app sheet only
    // (a widget sheet would override the user's theme), so bold and taller instead
    QFont bold = button->font();
    bold.setBold(true);
    button->setFont(bold);
    button->setMinimumHeight(button->sizeHint().height() + 12);
    ui.middleLayout->addWidget(button);
    QObject::connect(button, &QPushButton::clicked, &dialog, [&, a] {
      action = "ok";
      pick = a;
      dialog.accept();
    });
  }
  QObject::connect(ui.manualBtn, &QPushButton::clicked, &dialog, [&] { action = "manual"; dialog.accept(); });
  QObject::connect(ui.cancelBtn, &QPushButton::clicked, &dialog, &QDialog::reject);
  dialog.adjustSize();
  dialog.exec();
  return {{"action", action}, {"pick", pick}};
}

QJsonObject InstallDialogs::fomod(const QJsonObject& r)
{
  if (!m_fomod) {
    m_fomod = new FomodDialog(m_parent);
  }
  return m_fomod->wait(r);
}

// installer_manual's InstallDialog: untick what not to install, "Set as <data> directory"
QJsonObject InstallDialogs::manual(const QJsonObject& r)
{
  QDialog dialog(m_parent);
  Ui::InstallDialog ui;
  ui.setupUi(&dialog);
  fillNames(ui.nameCombo, r);
  const QString data = r["data_dir"].toString("Data").toLower();
  auto* tree = ui.treeContent;
  tree->setHeaderHidden(true);
  tree->setContextMenuPolicy(Qt::CustomContextMenu);

  QHash<QString, QTreeWidgetItem*> items;
  auto* top = new QTreeWidgetItem(tree, {"<" + data + ">"});
  top->setData(0, Qt::UserRole, QString());
  for (const auto& p : strings(r["paths"])) {
    const bool dir = p.endsWith('/');
    const QString path = dir ? p.chopped(1) : p;
    const int cut = path.lastIndexOf('/');
    QTreeWidgetItem* parent = cut < 0 ? top : items.value(path.left(cut), top);
    auto* item = new QTreeWidgetItem(parent, {path.mid(cut + 1)});
    item->setData(0, Qt::UserRole, p);
    item->setFlags(item->flags() | Qt::ItemIsUserCheckable | Qt::ItemIsAutoTristate);
    item->setCheckState(0, Qt::Checked);
    item->setIcon(0, QApplication::style()->standardIcon(dir ? QStyle::SP_DirIcon : QStyle::SP_FileIcon));
    if (dir) {
      items.insert(path, item);
    }
  }
  top->setExpanded(true);

  QString dataRoot;
  auto showProblem = [&] {
    if (!dataRoot.isEmpty()) {
      ui.problemLabel->setText(tr("<%1> is \"%2\".").arg(data, dataRoot));
      ui.problemLabel->setStyleSheet(QString());
    } else if (r["valid"].toBool()) {
      ui.problemLabel->setText(tr("The content of <%1> looks valid.").arg(data));
      ui.problemLabel->setStyleSheet("color: darkGreen;");
    } else {
      ui.problemLabel->setText(tr("The content of <%1> does not look valid.").arg(data));
      ui.problemLabel->setStyleSheet("color: red;");
    }
  };
  showProblem();
  QObject::connect(tree, &QTreeWidget::customContextMenuRequested, &dialog, [&](const QPoint& pos) {
    auto* item = tree->itemAt(pos);
    if (!item) {
      return;
    }
    const QString path = item->data(0, Qt::UserRole).toString();
    QMenu menu;
    if (path.endsWith('/')) {
      menu.addAction(tr("Set as <%1> directory").arg(data), [&, path] {
        dataRoot = path.chopped(1);
        top->setText(0, "<" + data + "> = " + dataRoot);
        showProblem();
      });
    }
    if (!dataRoot.isEmpty()) {
      menu.addAction(tr("Unset <%1> directory").arg(data), [&] {
        dataRoot.clear();
        top->setText(0, "<" + data + ">");
        showProblem();
      });
    }
    if (!menu.isEmpty()) {
      menu.exec(tree->viewport()->mapToGlobal(pos));
    }
  });
  bool ok = false;
  QObject::connect(ui.okButton, &QPushButton::clicked, &dialog, [&] { ok = true; dialog.accept(); });
  QObject::connect(ui.cancelButton, &QPushButton::clicked, &dialog, &QDialog::reject);
  dialog.exec();

  QJsonArray removed;
  for (auto it = QTreeWidgetItemIterator(tree); *it; ++it) {
    // a folder left fully unticked is removed whole; its children need no entry
    if (*it != top && (*it)->checkState(0) == Qt::Unchecked &&
        ((*it)->parent() == top || (*it)->parent()->checkState(0) != Qt::Unchecked)) {
      removed.append((*it)->data(0, Qt::UserRole).toString());
    }
  }
  return {{"action", ok ? "ok" : "cancel"},
          {"name", ui.nameCombo->currentText()},
          {"data_root", dataRoot},
          {"removed", removed}};
}

// RootSorter::confirm (Leet change 4)
QJsonObject InstallDialogs::rootSort(const QJsonObject& r)
{
  const QString base = r["base"].toString();
  auto join = [&](const QString& name) { return base.isEmpty() ? name : base + "/" + name; };
  QString list = tr("Move to %1:").arg("Root/") + "\n";
  for (const auto& n : strings(r["to_root"])) {
    list += "    " + n + "\n";
  }
  if (r["data_split"].toBool()) {
    list += "\n" + tr("Install the contents of %1 as the mod's Data files.").arg(join("Data/")) + "\n";
  }
  const QStringList dropped = strings(r["dropped"]);
  if (!dropped.isEmpty()) {
    list += "\n" + tr("Not installed:") + "\n";
    for (const auto& p : dropped) {
      list += "    " + p + "\n";
    }
  }
  const QStringList unsure = strings(r["unsure"]);
  if (!unsure.isEmpty()) {
    list += "\n" + tr("Left in place (might be Data inis):") + "\n";
    for (const auto& n : unsure) {
      list += "    " + n + "\n";
    }
  }

  QDialog dialog(m_parent);
  dialog.setWindowTitle(tr("Root files"));
  auto* label = new QLabel(tr("<b>%1</b> has files that belong in the game folder, not in Data. Move them into "
                              "the mod's %2 folder? Files in %2 are placed in the game folder when a program "
                              "is launched.")
                               .arg(r["mod_name"].toString().toHtmlEscaped(), "Root"));
  label->setWordWrap(true);
  auto* text = new QPlainTextEdit(list);
  text->setReadOnly(true);
  text->setLineWrapMode(QPlainTextEdit::NoWrap);
  auto* buttons = new QDialogButtonBox;
  buttons->addButton(tr("Move"), QDialogButtonBox::AcceptRole)->setDefault(true);
  buttons->addButton(tr("Don't move"), QDialogButtonBox::RejectRole);
  QObject::connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
  QObject::connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
  auto* layout = new QVBoxLayout(&dialog);
  layout->addWidget(label);
  layout->addWidget(text);
  layout->addWidget(buttons);
  dialog.resize(560, 420);
  return {{"move", dialog.exec() == QDialog::Accepted}};
}

// QueryOverwriteDialog, then the rename question of InstallationManager::testOverwrite
QJsonObject InstallDialogs::clash(const QJsonObject& r)
{
  QDialog dialog(m_parent);
  Ui::QueryOverwriteDialog ui;
  ui.setupUi(&dialog);
  ui.iconLabel->setPixmap(QApplication::style()->standardIcon(QStyle::SP_MessageBoxQuestion).pixmap(128));
  QString action = "cancel";
  for (auto [button, name] : {std::pair{ui.mergeBtn, "merge"}, {ui.replaceBtn, "replace"}, {ui.renameBtn, "rename"}}) {
    QObject::connect(button, &QPushButton::clicked, &dialog, [&, name] {
      action = name;
      dialog.accept();
    });
  }
  QObject::connect(ui.cancelBtn, &QPushButton::clicked, &dialog, &QDialog::reject);
  dialog.exec();
  QJsonObject a{{"action", action}, {"backup", ui.backupBox->isChecked()}};
  if (action == "rename") {
    bool ok = false;
    const QString name = QInputDialog::getText(m_parent, tr("Mod Name"), tr("Name"), QLineEdit::Normal,
                                               r["name"].toString(), &ok);
    if (!ok || name.trimmed().isEmpty()) {
      a["action"] = "cancel";
    }
    a["name"] = name;
  }
  return a;
}

void startInstall(QWidget* parent, const QString& root, const QString& profile, const QString& archive,
                  const QString& under, std::function<void(const QString&, const QString&)> done)
{
  auto* host = new InstallDialogs(parent);
  auto result = std::make_shared<std::pair<QString, QString>>();
  QThread* worker = QThread::create([=] {
    try {
      auto report = install_archive(root.toStdString(), profile.toStdString(), archive.toStdString(),
                                    under.toStdString(), *host);
      result->first = QString::fromUtf8(report.data(), qsizetype(report.size()));
    } catch (const rust::Error& e) {
      result->second = QString::fromUtf8(e.what());
    }
  });
  QObject::connect(worker, &QThread::finished, host, [=] {
    done(result->first, result->second);
    worker->deleteLater();
    host->deleteLater();
  });
  worker->start();
}

}  // namespace strata

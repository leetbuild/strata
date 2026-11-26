#include "dialogs.h"

#include "crashlogviewer.h"
#include "common.h"
#include "deleteconfirm.h"
#include "executablesdialog.h"
#include "inieditor.h"
#include "problemsdialog.h"
#include "profilesdialog.h"
#include "settingsdialog.h"
#include "strata_bridge/bridge_builtins.h"

#include "geometrysaver.h"

#include <QCoreApplication>
#include <QLocale>
#include <QMessageBox>
#include <QTabWidget>
#include <QTimer>

namespace strata::modlist
{

bool showProfilesDialog(QWidget* parent, const QString& root, QString* selected)
{
  ProfilesDialog dialog(root, parent);
  {
    GeometrySaver geometry(root, &dialog);  // MO2's GeometrySaver in exec()
    dialog.exec();
  }
  if (selected && dialog.selected()) {
    *selected = *dialog.selected();
  }
  return dialog.changed() || dialog.selected().has_value();
}

bool showExecutablesDialog(QWidget* parent, const QString& root, const QString& profile)
{
  ExecutablesDialog dialog(root, profile, parent);
  {
    GeometrySaver geometry(root, &dialog);  // MO2's GeometrySaver in exec()
    dialog.exec();
  }
  return dialog.changed();
}

bool showSettingsDialog(QWidget* parent, const QString& root)
{
  SettingsDialog dialog(root, parent);
  {
    GeometrySaver geometry(root, &dialog);  // MO2's GeometrySaver in exec()
    dialog.exec();
  }
  return dialog.changed();
}

bool showIniEditor(QWidget* parent, const QString& root, const QString& profile)
{
  IniEditorDialog dialog(root, profile, parent);
  dialog.exec();
  return dialog.changed();
}

bool showProblemsDialog(QWidget* parent, const QString& root, const QString& profile)
{
  ProblemsDialog dialog(root, profile, parent);
  dialog.exec();
  return dialog.changed();
}


void showCrashLogViewer(QWidget* parent, const QString& root, const QString& profile)
{
  CrashLogViewer dialog(root, profile, parent);
  dialog.exec();
}

void openDialogByName(QWidget* parent, const QString& name, const QString& root, const QString& profile)
{
  const QString shot = qEnvironmentVariable("STRATA_SHELL_DIALOG_SCREENSHOT");
  if (shot.isEmpty()) {
    if (name == "profiles") {
      showProfilesDialog(parent, root, nullptr);
    } else if (name == "executables") {
      showExecutablesDialog(parent, root, profile);
    } else if (name == "settings") {
      showSettingsDialog(parent, root);
    } else if (name == "inieditor") {
      showIniEditor(parent, root, profile);
    } else if (name == "problems") {
      showProblemsDialog(parent, root, profile);
    } else if (name == "crashlogs") {
      showCrashLogViewer(parent, root, profile);
    } else if (name == "delete") {
      askDeleteChoice(parent, root, QObject::tr("Confirm"), QObject::tr("Are you sure you want to remove \"%1\"?").arg("Mod"), "removeMod");
    }
    return;
  }

  QDialog* dialog = nullptr;
  if (name == "profiles") {
    dialog = new ProfilesDialog(root, parent);
  } else if (name == "executables") {
    dialog = new ExecutablesDialog(root, profile, parent);
  } else if (name.startsWith("settings")) {
    auto* settings = new SettingsDialog(root, parent);
    // "settings:<tab>" picks a tab for the picture
    if (const int tab = name.section(':', 1).toInt(); tab > 0) {
      settings->findChild<QTabWidget*>("tabWidget")->setCurrentIndex(tab);
    }
    dialog = settings;
  } else if (name == "inieditor") {
    dialog = new IniEditorDialog(root, profile, parent);
  } else if (name == "problems") {
    dialog = new ProblemsDialog(root, profile, parent);
  } else if (name == "crashlogs") {
    dialog = new CrashLogViewer(root, profile, parent);
  } else {
    dialog = new DeleteConfirmDialog(parent, QObject::tr("Confirm"),
                                     QObject::tr("Are you sure you want to remove \"%1\"?").arg("SkyUI"));
  }
  dialog->setAttribute(Qt::WA_DeleteOnClose);
  dialog->show();
  const int delay = qEnvironmentVariableIntegerValue("STRATA_SHELL_SCREENSHOT_DELAY").value_or(800);
  QTimer::singleShot(delay, dialog, [dialog, shot] {
    const bool saved = dialog->grab().save(shot);
    QCoreApplication::exit(saved ? 0 : 1);
  });
}

}  // namespace strata::modlist

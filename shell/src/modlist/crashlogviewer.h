#pragma once

// The Crash Log Viewer (the third-party crashlogtools plugin, built in): the crash-*.log files of
// the crash log folder, newest first; Open (bold, also a double click or Enter) hands a log
// to the desktop's text editor, Delete moves the selected logs to the trash. Opening it
// labels the logs first when the Crash Log Labeler is on (the plugin did that when MO2's
// window opened); "Update names database" downloads the labeler's database.

#include <QDialog>

class QLabel;
class QListWidget;

namespace strata::modlist
{

class CrashLogViewer : public QDialog
{
  Q_OBJECT

public:
  CrashLogViewer(const QString& root, const QString& profile, QWidget* parent = nullptr);

private:
  void reload();
  void openSelected();
  void deleteSelected();

  QString m_root;
  QString m_profile;
  QListWidget* m_list;
  QLabel* m_status;
};

}  // namespace strata::modlist

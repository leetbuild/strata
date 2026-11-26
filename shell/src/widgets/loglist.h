// MO2's LogList (the log dock), which mainwindow.ui names. The class name must stay: MO2's
// stylesheets select on it. Shows the program's Qt messages (qDebug / qInfo / qWarning...)
// as MO2's LogModel does: time, level icon, message.
#pragma once

#include <QStandardItemModel>
#include <QTimer>
#include <QTreeView>

class LogList : public QTreeView
{
  Q_OBJECT

public:
  explicit LogList(QWidget* parent = nullptr);

  // Routes Qt's messages into the log (and still to stderr). Call once, after the
  // application exists.
  static void installMessageHandler();

  // The model every LogList shows.
  static QStandardItemModel& model();

private:
  QTimer m_scroll;
};

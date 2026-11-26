#include "loglist.h"

#include <QCoreApplication>
#include <QHeaderView>
#include <QIcon>
#include <QPointer>
#include <QTime>

namespace
{

QtMessageHandler previousHandler = nullptr;

// LogModel::data, Qt::DecorationRole.
QIcon levelIcon(QtMsgType type)
{
  switch (type) {
  case QtWarningMsg:
    return QIcon(":/MO/gui/warning");
  case QtCriticalMsg:
  case QtFatalMsg:
    return QIcon(":/MO/gui/problem");
  case QtDebugMsg:
    return QIcon(":/MO/gui/debug");
  case QtInfoMsg:
    return QIcon(":/MO/gui/information");
  }
  return {};
}

void append(QtMsgType type, const QString& message)
{
  auto& model = LogList::model();
  auto* time  = new QStandardItem(QTime::currentTime().toString("hh:mm:ss.zzz"));
  auto* icon  = new QStandardItem(levelIcon(type), QString());
  auto* text  = new QStandardItem(message);
  for (auto* item : {time, icon, text}) {
    item->setEditable(false);
  }
  model.appendRow({time, icon, text});
}

void handler(QtMsgType type, const QMessageLogContext& context, const QString& message)
{
  if (previousHandler) {
    previousHandler(type, context, message);
  }
  // messages may come from any thread: the model lives on the UI thread
  if (auto* app = QCoreApplication::instance()) {
    QMetaObject::invokeMethod(app, [type, message] { append(type, message); },
                              Qt::QueuedConnection);
  }
}

}  // namespace

QStandardItemModel& LogList::model()
{
  static QStandardItemModel model(0, 3);
  return model;
}

void LogList::installMessageHandler()
{
  previousHandler = qInstallMessageHandler(handler);
}

LogList::LogList(QWidget* parent) : QTreeView(parent)
{
  setModel(&LogList::model());
  setRootIsDecorated(false);
  setHeaderHidden(true);
  header()->setMinimumSectionSize(0);
  header()->resizeSection(1, 20);
  header()->setSectionResizeMode(0, QHeaderView::ResizeToContents);
  header()->setStretchLastSection(true);

  m_scroll.setSingleShot(true);
  m_scroll.setInterval(50);
  connect(&m_scroll, &QTimer::timeout, this, &QTreeView::scrollToBottom);
  connect(&LogList::model(), &QAbstractItemModel::rowsInserted, &m_scroll,
          qOverload<>(&QTimer::start));
}

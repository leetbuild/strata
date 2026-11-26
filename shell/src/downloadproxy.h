// The Downloads tab's sort/filter proxy. A drag of a row carries what MO2's
// DownloadList::mimeData gives (the text "download") plus the archive's file name, so a drop
// on the mod list installs it there (ModListDropInfo::isDownloadDrop).
#pragma once

#include <QMimeData>
#include <QSortFilterProxyModel>

class DownloadProxy : public QSortFilterProxyModel
{
public:
  using QSortFilterProxyModel::QSortFilterProxyModel;

  QMimeData* mimeData(const QModelIndexList& indexes) const override
  {
    QMimeData* data = QSortFilterProxyModel::mimeData(indexes);
    if (data && !indexes.isEmpty()) {
      data->setText(QStringLiteral("download"));
      // the file name is column 0's tool tip (MainWindow::loadDownloads)
      data->setData(QStringLiteral("application/x-strata-download"),
                    indexes.first().siblingAtColumn(0).data(Qt::ToolTipRole).toString().toUtf8());
    }
    return data;
  }
};

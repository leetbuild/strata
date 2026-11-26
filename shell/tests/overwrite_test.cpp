// Overwrite: its dialog (a double click: New Folder, Delete), its context menu (Create
// Mod..., Clear Overwrite...), and dialog sizes stored as MO2's GeometrySaver does.

#include "tests.h"

#include <QDir>
#include <QFileSystemModel>
#include <QInputDialog>
#include <QMessageBox>
#include <QTest>

using namespace drive;

class OverwriteTest : public QObject
{
  Q_OBJECT

  WindowFixture w;

  QString overwrite() const { return w.root + "/overwrite"; }

private slots:
  void initTestCase()
  {
    QVERIFY(w.open(makeInstance("overwrite")));
    QTRY_VERIFY_WITH_TIMEOUT(w.mod("Overwrite").isValid(), 15000);
  }

  void cleanupTestCase() { w.close(); }

  void theOverwriteDialogMakesFoldersAndDeletesToTheTrash()
  {
    QVERIFY(QDir(overwrite() + "/meshes").exists());
    bool done = false;
    whenModal(
        [&done](QWidget* d) {
          auto* view = d->findChild<QTreeView*>("filesView");
          auto* model = qobject_cast<QFileSystemModel*>(view->model());
          // QFileSystemModel reads the folder on its own thread
          QPointer<QWidget> dialog(d);
          when([=] { return dialog && find(model, Qt::DisplayRole, "meshes", view->rootIndex()).isValid(); }, [=, &done] {
            // New Folder from the menu on the empty part of the list
            whenMenu({"New Folder"});
            const QPoint at(5, view->viewport()->height() - 5);
            QContextMenuEvent menu(QContextMenuEvent::Mouse, at, view->viewport()->mapToGlobal(at));
            QApplication::sendEvent(view->viewport(), &menu);
            // once the list shows the new folder (QFileSystemModel's watcher, on its own
            // thread), "meshes" selected, Delete, Yes; closed when it is gone from the list
            when([=] { return dialog && find(model, Qt::DisplayRole, "New Folder", view->rootIndex()).isValid(); },
                 [=, &done] {
                   const QModelIndex meshes = find(model, Qt::DisplayRole, "meshes", view->rootIndex());
                   view->setFocus();
                   clickRow(view, meshes);
                   const auto rows = view->selectionModel()->selectedRows();
                   QVERIFY(rows.size() == 1 && rows.first().data().toString() == "meshes");
                   whenModal([](QWidget* m) { QTest::mouseClick(qobject_cast<QMessageBox*>(m)->button(QMessageBox::Yes), Qt::LeftButton); },
                             [](QWidget* m) { return qobject_cast<QMessageBox*>(m) != nullptr; });
                   QTest::keyClick(view, Qt::Key_Delete);
                   when([=] { return dialog && !find(model, Qt::DisplayRole, "meshes", view->rootIndex()).isValid(); },
                        [=, &done] {
                          done = true;
                          shoot(dialog, "overwrite-dialog");
                          clickButton(dialog, QDialogButtonBox::Close);
                        });
                 });
          });
        },
        named("OverwriteInfoDialog"));
    const QModelIndex row = w.mod("Overwrite");
    doubleClick(w.modList()->viewport(), onRow(w.modList(), row));
    QTRY_VERIFY(done);
    QVERIFY(QDir(overwrite() + "/New Folder").exists());
    QVERIFY(!QDir(overwrite() + "/meshes").exists());
    QVERIFY(QDir(stage() + "/data/Trash/files").entryList(QDir::NoDotAndDotDot | QDir::AllEntries).contains("meshes"));
    // MO2's GeometrySaver: the dialog's size is in [Geometry]
    QTRY_VERIFY(w.ini().contains("OverwriteInfoDialog_geometry=@ByteArray("));
  }

  void createModMovesOverwriteIntoANewMod()
  {
    QVERIFY(QFile::exists(overwrite() + "/SKSE/Plugins/written.log"));
    whenMenu({"Create Mod..."});
    whenModalOf<QInputDialog>([](QInputDialog* d) {
      QTest::keyClicks(d->findChild<QLineEdit*>(), "Written Logs");
      clickButton(d, QDialogButtonBox::Ok);
    });
    contextMenu(w.modList(), w.mod("Overwrite"));
    QTRY_VERIFY(w.mod("Written Logs").isValid());
    QVERIFY(QFile::exists(w.root + "/mods/Written Logs/SKSE/Plugins/written.log"));
    QCOMPARE(QDir(overwrite()).entryList(QDir::NoDotAndDotDot | QDir::AllEntries | QDir::Hidden), QStringList());
  }

  void clearOverwriteEmptiesIt()
  {
    QDir().mkpath(overwrite() + "/textures");
    QFile f(overwrite() + "/textures/new.dds");
    QVERIFY(f.open(QIODevice::WriteOnly));
    f.close();
    whenMenu({"Clear Overwrite..."});
    whenModalOf<QMessageBox>([](QMessageBox* m) { QTest::mouseClick(m->button(QMessageBox::Ok), Qt::LeftButton); });
    contextMenu(w.modList(), w.mod("Overwrite"));
    QTRY_COMPARE(QDir(overwrite()).entryList(QDir::NoDotAndDotDot | QDir::AllEntries | QDir::Hidden), QStringList());
  }

  void dialogSizesAreRestored()
  {
    // the profiles dialog opened twice: the second time at the size it was left
    QSize first;
    whenModal(
        [&first](QWidget* d) {
          d->resize(d->width() + 77, d->height() + 33);
          first = d->size();
          QTest::mouseClick(d->findChild<QPushButton*>("close"), Qt::LeftButton);
        },
        named("ProfilesDialog"));
    w.window->findChild<QAction*>("actionAdd_Profile")->trigger();
    QTRY_VERIFY(w.ini().contains("ProfilesDialog_geometry=@ByteArray("));
    QSize second;
    whenModal(
        [&second](QWidget* d) {
          second = d->size();
          QTest::mouseClick(d->findChild<QPushButton*>("close"), Qt::LeftButton);
        },
        named("ProfilesDialog"));
    w.window->findChild<QAction*>("actionAdd_Profile")->trigger();
    QTRY_VERIFY(second.isValid());
    QCOMPARE(second, first);
  }
};

std::unique_ptr<QObject> drive::makeOverwriteTest()
{
  return std::make_unique<OverwriteTest>();
}

#include "overwrite_test.moc"

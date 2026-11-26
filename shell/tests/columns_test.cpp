// First-start column widths (ColumnFit): with no saved header, at the window's default size,
// both lists show every default-visible column inside the view, without a horizontal
// scrollbar, and the name column keeps at least 40% of the width. Long categories and
// versions make the other columns want more than there is (a real list's case on a
// small screen, where the name column was squeezed and Priority pushed out of view).

#include "tests.h"

#include <QDirIterator>
#include <QApplication>
#include <QCheckBox>
#include <QHeaderView>
#include <QMenu>
#include <QTimer>
#include <QScrollBar>
#include <QTest>

#include "pluginstab.h"

using namespace drive;

namespace
{

QString describe(QTreeView* view)
{
  QStringList parts;
  const auto* header = view->header();
  for (int c = 0; c < header->count(); ++c) {
    if (!header->isSectionHidden(c)) {
      parts << QString("%1=%2").arg(view->model()->headerData(c, Qt::Horizontal).toString()).arg(header->sectionSize(c));
    }
  }
  return QString("viewport %1: %2").arg(view->viewport()->width()).arg(parts.join(", "));
}

void checkFits(QTreeView* view, int nameColumn)
{
  const auto* header = view->header();
  const int viewport = view->viewport()->width();
  int last = -1;
  for (int v = header->count() - 1; v >= 0 && last < 0; --v) {
    if (!header->isSectionHidden(header->logicalIndex(v))) {
      last = header->logicalIndex(v);
    }
  }
  const QString what = describe(view);
  QVERIFY2(!view->horizontalScrollBar()->isVisible(), qPrintable("a horizontal scrollbar: " + what));
  QVERIFY2(header->sectionViewportPosition(last) + header->sectionSize(last) <= viewport,
           qPrintable("the last column ends outside the view: " + what));
  QVERIFY2(header->sectionSize(nameColumn) * 10 >= viewport * 4, qPrintable("the name column is under 40%: " + what));
}

}  // namespace

class ColumnsTest : public QObject
{
  Q_OBJECT

  QString m_root;

private slots:
  void initTestCase()
  {
    m_root = makeInstance("columns");
    // long categories and versions, as real mods have them
    QFile cats(m_root + "/categories.dat");
    QVERIFY(cats.open(QIODevice::WriteOnly));
    cats.write("1|Visuals and Graphics Overhauls|0\n2|Bug Fixes|0\n");
    cats.close();
    QDirIterator it(m_root + "/mods", {"meta.ini"}, QDir::Files, QDirIterator::Subdirectories);
    while (it.hasNext()) {
      QFile meta(it.next());
      QVERIFY(meta.open(QIODevice::Append));
      meta.write("category=\"1,2\"\r\n");
      meta.close();
      QString text = fileText(meta.fileName());
      text.replace("version=\r\n", "version=1.17.1.0.4712\r\n");
      QVERIFY(meta.open(QIODevice::WriteOnly | QIODevice::Truncate));
      meta.write(text.toUtf8());
    }
  }

  // the window as main.cpp opens it: no resize, the form's own size
  void bothListsFitAtTheDefaultSize()
  {
    Style style;
    MainWindow window(style);
    QVERIFY(window.openInstance(m_root, {}));
    window.show();
    auto* mods    = window.findChild<QTreeView*>("modList");
    auto* plugins = window.findChild<QTreeView*>("espList");
    QTRY_VERIFY_WITH_TIMEOUT(!window.findChild<PluginsTab*>()->loading(), 10000);
    QTest::qWait(300);  // the fits run after the first layout and after the plugin fill
    checkFits(mods, modlist::ColName);
    checkFits(plugins, 0);
    // a wider window: still fitted (the fit follows the width until the header is touched)
    window.resize(window.width() + 400, window.height());
    QTest::qWait(300);
    checkFits(mods, modlist::ColName);
    window.close();
  }

  // MO2's header menu (setCustomizableColumns): a right click on a list's header lists its
  // columns with checkboxes; unticking one hides it, ticking shows it again
  void aRightClickOnTheHeaderShowsAndHidesColumns()
  {
    Style style;
    MainWindow window(style);
    QVERIFY(window.openInstance(m_root, {}));
    window.show();
    for (const char* name : {"modList", "espList", "downloadView", "dataTree", "rootTree"}) {
      auto* view   = window.findChild<QTreeView*>(name);
      auto* header = view->header();
      QCOMPARE(header->contextMenuPolicy(), Qt::CustomContextMenu);
    }
    auto* header = window.findChild<QTreeView*>("modList")->header();
    const int column = 1;
    QVERIFY(!header->isSectionHidden(column));
    auto toggle = [&](int expectedBoxes) {
      int boxes = -1;
      QTimer::singleShot(100, [&boxes, column] {
        auto* menu = qobject_cast<QMenu*>(QApplication::activePopupWidget());
        if (!menu) {
          return;
        }
        const auto all = menu->findChildren<QCheckBox*>();
        boxes = int(all.size());
        if (column - 1 < all.size()) {
          all[column - 1]->click();  // the first column has no box
        }
        menu->close();
      });
      emit header->customContextMenuRequested(QPoint(10, 5));
      QCOMPARE(boxes, expectedBoxes);
    };
    toggle(header->count() - 1);
    QVERIFY(header->isSectionHidden(column));
    toggle(header->count() - 1);
    QVERIFY(!header->isSectionHidden(column));
    window.close();
  }
};

std::unique_ptr<QObject> drive::makeColumnsTest()
{
  return std::make_unique<ColumnsTest>();
}

#include "columns_test.moc"

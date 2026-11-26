#pragma once

// The window's layout in the instance's ModOrganizer.ini, under MO2's keys, so MO2 and
// Strata restore each other's layout (MainWindow::readSettings / storeSettings,
// GeometrySettings, WidgetSettings in MO2's settings.cpp).
//
// Leet change 2, extended to everything: any change (a header section resized by the user,
// moved or sorted, a splitter moved, the window moved or resized, a tab or executable
// picked, a bar shown or hidden) starts one 500 ms timer; when it fires, every value is
// stored in one write, and only when the file's bytes change.

#include "strata_bridge/bridge.h"

#include <QHash>
#include <QObject>
#include <QSet>
#include <QPointer>
#include <QTimer>

#include <functional>
#include <optional>
#include <vector>

class QComboBox;
class QHeaderView;
class QMainWindow;
class QSplitter;
class QTabWidget;
class QWidget;

class LayoutState : public QObject
{
  Q_OBJECT

public:
  explicit LayoutState(QMainWindow* window);

  // What to save, by the names MO2 gives them (widgetNameWithTopLevel: "MainWindow_" +
  // objectName). Register everything, then call restore() once the models are set.
  void addHeader(QHeaderView* header);
  // The same, also reading another program's header of this list: `map[i]` is this list's
  // column for that program's column i (-1: a column this list doesn't have). A saved state
  // with `map.size()` sections is read through it: sizes, hidden columns, order and the sort
  // column carry over (Fluorine's 13-column mod list and 8-column plugin list).
  void addHeader(QHeaderView* header, std::vector<std::vector<int>> foreignMaps);
  // Whether restore() set the header from the ini (its own state or a mapped one); when not,
  // the window gives it first-start widths.
  bool restored(const QHeaderView* header) const { return m_restored.contains(header); }
  void addSplitter(QSplitter* splitter);
  void addVisibility(QWidget* widget);
  void addIndex(QTabWidget* tabs);
  void addIndex(QComboBox* combo);
  // A string list in [Widgets] (the mod list's open separators); `get` returns nothing
  // while there is nothing meaningful to save (a flat list has no open separators).
  void addList(const QString& key, std::function<std::optional<QStringList>()> get);

  // While one lives, changes are the program's own (a model swapped in, a state restored),
  // not the user's: nothing is marked for saving.
  class Quiet
  {
  public:
    explicit Quiet(LayoutState& state) : m_state(state) { ++m_state.m_quiet; }
    ~Quiet() { --m_state.m_quiet; }
    Quiet(const Quiet&)            = delete;
    Quiet& operator=(const Quiet&) = delete;

  private:
    LayoutState& m_state;
  };

  // The user changed one of the lists (addList). Before the watching starts (the first
  // second) the lists are left out of the baseline, so such a change is still saved.
  void touch()
  {
    m_listsTouched = true;
    changed();
  }

  // Restores what the ini holds, then starts watching for changes.
  void restore(const strata::Session& session);
  // Where to save from now on (after a profile switch the session object changes).
  void setSession(const strata::Session* session) { m_session = session; }

  // Writes now if anything changed since the last write (on close).
  void flush();

protected:
  bool eventFilter(QObject* watched, QEvent* event) override;

private:
  struct Value
  {
    const char* section;
    QString key;
    QByteArray bytes;
    std::uint8_t kind;  // strata::SettingValue::kind
  };
  using Values = std::vector<Value>;

  // Every value as it would be stored now.
  Values collect() const;
  void changed();
  void headerChanged(QHeaderView* header);

  QMainWindow* m_window;
  const strata::Session* m_session = nullptr;
  bool m_connected = false;  // the change signals are hooked up
  bool m_armed     = false;  // changes count (not during the start)
  bool m_listsTouched = false;
  int m_quiet      = 0;      // Quiet objects alive
  QTimer m_timer;
  QSet<QHeaderView*> m_dirtyHeaders;     // changed by the user since the start
  QHash<QString, QByteArray> m_baseline;  // "section/key" → bytes as last restored or written
  std::vector<QPointer<QHeaderView>> m_headers;
  QHash<const QHeaderView*, std::vector<std::vector<int>>> m_foreign;
  QSet<const QHeaderView*> m_restored;
  std::vector<QPointer<QSplitter>> m_splitters;
  std::vector<QPointer<QWidget>> m_visibility;
  std::vector<QPointer<QTabWidget>> m_tabs;
  std::vector<QPointer<QComboBox>> m_combos;
  std::vector<std::pair<QString, std::function<std::optional<QStringList>()>>> m_lists;
};

#pragma once

// The ini editor (MO2's tool_inieditor in uibase's TextViewer, textviewer.ui): one tab per
// game ini of the profile, each with a Save button; closing asks about unsaved tabs. Saving
// goes through the core (strata::builtins::ini_save: CRLF as MO2 writes, only when changed).

#include <QDialog>
#include <QSet>

#include <memory>

class QTextEdit;

namespace Ui
{
class TextViewer;
}

namespace strata::modlist
{

class IniEditorDialog : public QDialog
{
  Q_OBJECT

public:
  IniEditorDialog(const QString& root, const QString& profile, QWidget* parent = nullptr);
  ~IniEditorDialog() override;

  // Whether a file was written.
  bool changed() const { return m_changed; }

protected:
  void closeEvent(QCloseEvent* event) override;
  bool eventFilter(QObject* object, QEvent* event) override;

private:
  void addTab(const QString& name, const QString& path, const QString& text, bool readOnly);
  // Saves the tab's editor; false when the user skipped it or it failed.
  bool save(QTextEdit* editor);
  void setShowWhitespace(bool on);
  void find();
  void findNext();
  QTextEdit* currentEditor() const;

  std::unique_ptr<Ui::TextViewer> ui;
  QString m_root;
  QString m_profile;
  QSet<QTextEdit*> m_modified;
  QString m_pattern;
  bool m_changed = false;
};

}  // namespace strata::modlist

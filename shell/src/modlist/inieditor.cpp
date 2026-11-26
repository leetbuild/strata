#include "inieditor.h"

#include "common.h"
#include "strata_bridge/bridge_builtins.h"
#include "ui_textviewer.h"

#include <QAction>
#include <QCloseEvent>
#include <QInputDialog>
#include <QKeyEvent>
#include <QMessageBox>
#include <QPushButton>
#include <QTextEdit>
#include <QVBoxLayout>

namespace strata::modlist
{

IniEditorDialog::IniEditorDialog(const QString& root, const QString& profile, QWidget* parent)
    : QDialog(parent), ui(new Ui::TextViewer), m_root(root), m_profile(profile)
{
  ui->setupUi(this);
  setWindowTitle(tr("INI Files"));
  // Leet's MOApplication::notify unticks "Show Whitespace" when a TextViewer first shows
  ui->showWhitespace->setChecked(false);
  connect(ui->showWhitespace, &QCheckBox::toggled, this, [this](bool on) { setShowWhitespace(on); });

  try {
    const auto view = strata::builtins::ini_editor_view(rs(root), rs(profile));
    ui->descriptionLabel->setText(qs(view.description));
    QStringList skipped;
    for (const auto& tab : view.tabs) {
      if (tab.skipped.empty()) {
        addTab(qs(tab.name), qs(tab.path), qs(tab.text), tab.read_only);
      } else if (qs(tab.skipped) == "too big") {
        // IniEditor::display: "Sorry, the file %1 is too large..."
        skipped << tr("%1 is too large to be handled by the INI Editor").arg(qs(tab.name));
      }
    }
    if (!skipped.isEmpty()) {
      ui->descriptionLabel->setText(ui->descriptionLabel->text() + "\n" + skipped.join("\n"));
    }
  } catch (const std::exception& e) {
    ui->descriptionLabel->setText(QString::fromUtf8(e.what()));
  }
}

IniEditorDialog::~IniEditorDialog() = default;

// TextViewer::addFile
void IniEditorDialog::addTab(const QString& name, const QString& path, const QString& text, bool readOnly)
{
  auto* page   = new QWidget();
  auto* layout = new QVBoxLayout(page);
  auto* editor = new QTextEdit(page);
  auto* document = new QTextDocument(editor);
  if (ui->showWhitespace->isChecked()) {
    QTextOption option;
    option.setFlags(QTextOption::ShowTabsAndSpaces);
    document->setDefaultTextOption(option);
  }
  editor->setDocument(document);
  editor->setAcceptRichText(false);
  editor->setPlainText(text);
  editor->setLineWrapMode(QTextEdit::NoWrap);
  editor->setObjectName("editorView");
  editor->setDocumentTitle(name);
  editor->setProperty("readOnlyFile", readOnly);
  editor->installEventFilter(this);
  QPalette palette = editor->palette();
  palette.setColor(QPalette::Inactive, QPalette::Highlight, palette.color(QPalette::Active, QPalette::Highlight));
  palette.setColor(QPalette::Inactive, QPalette::HighlightedText,
                   palette.color(QPalette::Active, QPalette::HighlightedText));
  editor->setPalette(palette);
  layout->addWidget(editor);

  auto* saveButton = new QPushButton(tr("Save"), page);
  layout->addWidget(saveButton);
  connect(saveButton, &QPushButton::clicked, this, [this, editor] {
    if (save(editor)) {
      m_modified.remove(editor);
    }
  });
  connect(editor, &QTextEdit::textChanged, this, [this, editor] { m_modified.insert(editor); });

  const int index = ui->editorTabs->addTab(page, name);
  ui->editorTabs->setTabToolTip(index, path);
}

// TextViewer::saveFile, with the read-only question as three buttons.
bool IniEditorDialog::save(QTextEdit* editor)
{
  quint8 readOnly = 0;
  if (editor->property("readOnlyFile").toBool()) {
    QMessageBox box(QMessageBox::Warning, tr("INI file is read-only"),
                    tr("Strata is attempting to write to \"%1\" which is currently set to read-only.")
                        .arg(editor->documentTitle()),
                    QMessageBox::NoButton, this);
    auto* clear = box.addButton(tr("Clear the read-only flag"), QMessageBox::YesRole);
    auto* once  = box.addButton(tr("Allow the write once"), QMessageBox::AcceptRole);
    once->setToolTip(tr("The file will be set to read-only again."));
    box.addButton(tr("Skip this file"), QMessageBox::RejectRole);
    box.exec();
    if (box.clickedButton() == clear) {
      readOnly = 1;
      editor->setProperty("readOnlyFile", false);
    } else if (box.clickedButton() == once) {
      readOnly = 2;
    } else {
      return false;
    }
  }
  try {
    m_changed |= strata::builtins::ini_save(rs(m_root), rs(m_profile), rs(editor->documentTitle()),
                                            rs(editor->toPlainText()), readOnly);
    return true;
  } catch (const std::exception& e) {
    QMessageBox::critical(this, tr("Error"), tr("failed to write to %1: %2").arg(editor->documentTitle(),
                                                                                 QString::fromUtf8(e.what())));
    return false;
  }
}

// TextViewer::closeEvent
void IniEditorDialog::closeEvent(QCloseEvent* event)
{
  for (int i = 0; i < ui->editorTabs->count(); ++i) {
    auto* editor = ui->editorTabs->widget(i)->findChild<QTextEdit*>("editorView");
    if (!m_modified.contains(editor)) {
      continue;
    }
    const auto answer = QMessageBox::question(this, tr("Save changes?"),
                                              tr("Do you want to save changes to %1?").arg(editor->documentTitle()),
                                              QMessageBox::Yes | QMessageBox::No | QMessageBox::Cancel);
    if (answer == QMessageBox::Cancel) {
      event->ignore();
      return;
    }
    if (answer == QMessageBox::Yes && save(editor)) {
      m_modified.remove(editor);
    }
  }
  QDialog::closeEvent(event);
}

void IniEditorDialog::setShowWhitespace(bool on)
{
  for (int i = 0; i < ui->editorTabs->count(); ++i) {
    auto* editor = ui->editorTabs->widget(i)->findChild<QTextEdit*>("editorView");
    QTextOption option = editor->document()->defaultTextOption();
    option.setFlags(on ? option.flags() | QTextOption::ShowTabsAndSpaces
                       : option.flags() & ~QTextOption::ShowTabsAndSpaces);
    editor->document()->setDefaultTextOption(option);
  }
}

QTextEdit* IniEditorDialog::currentEditor() const
{
  QWidget* page = ui->editorTabs->currentWidget();
  return page ? page->findChild<QTextEdit*>("editorView") : nullptr;
}

// Ctrl+F asks for the text (MO2: FindDialog), F3 finds the next match, wrapping once.
bool IniEditorDialog::eventFilter(QObject* object, QEvent* event)
{
  if (event->type() == QEvent::ShortcutOverride) {
    auto* key = static_cast<QKeyEvent*>(event);
    if (key->matches(QKeySequence::Find)) {
      find();
      return true;
    }
    if (key->matches(QKeySequence::FindNext)) {
      findNext();
      return true;
    }
  }
  return QDialog::eventFilter(object, event);
}

void IniEditorDialog::find()
{
  bool ok = false;
  const QString pattern = QInputDialog::getText(this, tr("Find"), tr("Find what:"), QLineEdit::Normal, m_pattern, &ok);
  if (ok) {
    m_pattern = pattern;
    findNext();
  }
}

// TextViewer::findNext
void IniEditorDialog::findNext()
{
  QTextEdit* editor = currentEditor();
  if (m_pattern.isEmpty() || !editor || editor->find(m_pattern)) {
    return;
  }
  const QTextCursor old = editor->textCursor();
  editor->moveCursor(QTextCursor::Start);
  if (!editor->find(m_pattern)) {
    editor->setTextCursor(old);
  }
}

}  // namespace strata::modlist

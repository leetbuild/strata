// MO2's TextEditor and HTMLEditor (texteditor.h), as modinfodialog.ui names them. The class
// names stay so MO2's stylesheets select on them; the editing itself is Qt's.
#pragma once

#include <QPlainTextEdit>
#include <QTextEdit>

class TextEditor : public QPlainTextEdit
{
  Q_OBJECT

public:
  using QPlainTextEdit::QPlainTextEdit;
};

class HTMLEditor : public QTextEdit
{
  Q_OBJECT

public:
  using QTextEdit::QTextEdit;
};

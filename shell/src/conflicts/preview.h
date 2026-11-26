// File previews (MO2's PreviewDialog with the preview_base and preview_dds plugins): images
// through Qt, DDS textures decoded by the core (strata_ui::conflicts::preview) and text files.
#pragma once

#include <QDialog>
#include <QImage>
#include <QString>

#include <memory>
#include <vector>

namespace Ui
{
class PreviewDialog;
}

namespace strata::conflicts
{

// Whether Strata can preview a file by its name (MO2 offers "Preview" only then).
bool canPreview(const QString& fileName);

// The picture of an image or DDS file; a null image with `error` set when it can't be read.
QImage loadImage(const QString& path, QString* error = nullptr, QString* format = nullptr);

// A thumbnail at most `size` pixels on its longer side, read cheaply (a DDS file's small mip
// level, a scaled read for other formats); null when it cannot be read. Any thread.
QImage loadThumbnail(const QString& path, int size);

// One file to preview: the real file and the mod it comes from.
struct PreviewVariant
{
  QString path;
  QString mod;
};

// MO2's preview dialog: one page per variant (every mod that has the file), the last one
// (the winner) shown first, previous / next to step through.
class PreviewDialog : public QDialog
{
  Q_OBJECT

public:
  PreviewDialog(const QString& shownName, const std::vector<PreviewVariant>& variants,
                QWidget* parent = nullptr);
  ~PreviewDialog() override;

private:
  void showVariant(int index);

  std::unique_ptr<Ui::PreviewDialog> ui;
  std::vector<PreviewVariant> m_variants;
};

// Opens the preview of one file of a mod.
void previewFile(QWidget* parent, const QString& path, const QString& mod);

// Opens the preview of a file of the merged tree with every loose copy the profile has
// (the Data tab's "Preview"). `path` is relative to Data, or to the game folder when
// `rootArea`. Returns false when no copy can be shown.
bool previewMergedFile(QWidget* parent, const QString& instanceRoot, const QString& profile,
                       const QString& path, bool rootArea);

}  // namespace strata::conflicts

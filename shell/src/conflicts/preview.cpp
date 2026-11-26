#include "conflicts/preview.h"
#include "geometrysaver.h"

#include "strata_bridge/bridge_conflicts.h"
#include "ui_previewdialog.h"

#include <QFileInfo>
#include <QPushButton>
#include <QImageReader>
#include <QLabel>
#include <QPlainTextEdit>
#include <QScrollArea>
#include <QVBoxLayout>

#include <cstring>

namespace strata::conflicts
{

namespace
{

QString qs(const ::rust::String& s)
{
  return QString::fromUtf8(s.data(), qsizetype(s.size()));
}

rust::String rs(const QString& s)
{
  const QByteArray utf8 = s.toUtf8();
  return ::rust::String(utf8.data(), std::size_t(utf8.size()));
}

QString kindOf(const QString& name)
{
  return qs(strata::conflicts::preview_kind(rs(name)));
}


// The page for one file: a scrollable picture, or read-only text, or the error.
QWidget* makePage(const QString& path)
{
  const QString kind = kindOf(QFileInfo(path).fileName());
  auto* page   = new QWidget;
  auto* layout = new QVBoxLayout(page);
  layout->setContentsMargins(0, 0, 0, 0);


  if (kind == "text") {
    auto* edit = new QPlainTextEdit;
    edit->setReadOnly(true);
    try {
      edit->setPlainText(qs(strata::conflicts::read_text(rs(path))));
    } catch (const std::exception& e) {
      edit->setPlainText(QString::fromUtf8(e.what()));
    }
    layout->addWidget(edit);
    return page;
  }

  QString error, format;
  const QImage image = loadImage(path, &error, &format);
  if (image.isNull()) {
    layout->addWidget(new QLabel(error));
    return page;
  }
  auto* picture = new QLabel;
  picture->setPixmap(QPixmap::fromImage(image));
  picture->setAlignment(Qt::AlignCenter);
  auto* scroll = new QScrollArea;
  scroll->setWidget(picture);
  scroll->setWidgetResizable(true);
  layout->addWidget(scroll);
  layout->addWidget(
      new QLabel(QObject::tr("%1 x %2, %3").arg(image.width()).arg(image.height()).arg(format)));
  return page;
}

}  // namespace

bool canPreview(const QString& fileName)
{
  return !kindOf(fileName).isEmpty();
}

QImage loadImage(const QString& path, QString* error, QString* format)
{
  if (kindOf(QFileInfo(path).fileName()) == "dds") {
    try {
      auto img = strata::conflicts::decode_dds(rs(path));
      QImage image(int(img.width), int(img.height), QImage::Format_ARGB32);
      const std::size_t row = std::size_t(img.width) * 4;
      for (int y = 0; y < int(img.height); ++y) {
        std::memcpy(image.scanLine(y), img.bgra.data() + std::size_t(y) * row, row);
      }
      if (format) {
        *format = qs(img.format);
      }
      return image;
    } catch (const std::exception& e) {
      if (error) {
        *error = QString::fromUtf8(e.what());
      }
      return {};
    }
  }
  QImageReader reader(path);
  QImage image = reader.read();
  if (image.isNull() && error) {
    *error = QObject::tr("cannot read %1: %2").arg(path, reader.errorString());
  }
  if (format) {
    *format = QString::fromLatin1(reader.format()).toUpper();
  }
  return image;
}

QImage loadThumbnail(const QString& path, int size)
{
  if (kindOf(QFileInfo(path).fileName()) == "dds") {
    try {
      auto img = strata::conflicts::decode_dds_thumbnail(rs(path), std::uint32_t(size));
      QImage image(int(img.width), int(img.height), QImage::Format_ARGB32);
      const std::size_t row = std::size_t(img.width) * 4;
      for (int y = 0; y < int(img.height); ++y) {
        std::memcpy(image.scanLine(y), img.bgra.data() + std::size_t(y) * row, row);
      }
      return image.scaled(size, size, Qt::KeepAspectRatio, Qt::SmoothTransformation);
    } catch (const std::exception&) {
      return {};
    }
  }
  QImageReader reader(path);
  const QSize full = reader.size();
  if (full.isValid() && (full.width() > size || full.height() > size)) {
    reader.setScaledSize(full.scaled(size, size, Qt::KeepAspectRatio));  // decoded small where the format can
  }
  return reader.read();
}

PreviewDialog::PreviewDialog(const QString& shownName, const std::vector<PreviewVariant>& variants,
                             QWidget* parent)
    : QDialog(parent), ui(new Ui::PreviewDialog), m_variants(variants)
{
  ui->setupUi(this);
  ui->nameLabel->setText(shownName);
  for (const auto& v : m_variants) {
    ui->variantsStack->addWidget(makePage(v.path));
  }
  const bool several = m_variants.size() > 1;
  ui->previousButton->setEnabled(several);
  ui->nextButton->setEnabled(several);
  connect(ui->previousButton, &QPushButton::clicked, this, [this] {
    const int n = ui->variantsStack->count();
    showVariant((ui->variantsStack->currentIndex() + n - 1) % n);
  });
  connect(ui->nextButton, &QPushButton::clicked, this, [this] {
    showVariant((ui->variantsStack->currentIndex() + 1) % ui->variantsStack->count());
  });
  connect(ui->closeButton, &QPushButton::clicked, this, &QDialog::accept);
  showVariant(int(m_variants.size()) - 1);
}

PreviewDialog::~PreviewDialog() = default;

void PreviewDialog::showVariant(int index)
{
  if (index < 0 || index >= int(m_variants.size())) {
    return;
  }
  ui->variantsStack->setCurrentIndex(index);
  ui->modLabel->setText(m_variants[std::size_t(index)].mod);
}

void previewFile(QWidget* parent, const QString& path, const QString& mod)
{
  PreviewDialog dialog(QFileInfo(path).fileName(), {{path, mod}}, parent);
  dialog.exec();
}

bool previewMergedFile(QWidget* parent, const QString& instanceRoot, const QString& profile,
                       const QString& path, bool rootArea)
{
  std::vector<PreviewVariant> variants;
  try {
    for (const auto& v :
         strata::conflicts::file_providers(rs(instanceRoot), rs(profile), rootArea, rs(path))) {
      variants.push_back({qs(v.real), qs(v.layer)});
    }
  } catch (const std::exception&) {
    return false;
  }
  if (variants.empty()) {
    return false;
  }
  PreviewDialog dialog(path, variants, parent);
  GeometrySaver geometry(instanceRoot, &dialog);  // MO2's GeometrySaver in exec()
  dialog.exec();
  return true;
}

}  // namespace strata::conflicts

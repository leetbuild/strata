#include "fomoddialog.h"

#include "ui_fomodinstallerdialog.h"

#include <QButtonGroup>
#include <QCheckBox>
#include <QCloseEvent>
#include <QGroupBox>
#include <QJsonArray>
#include <QLabel>
#include <QMessageBox>
#include <QRadioButton>
#include <QScrollArea>
#include <QVBoxLayout>

namespace strata
{

namespace
{

// The tool tips of FomodInstallerDialog::displayCurrentPage, by plugin type.
QString typeTip(const QString& type)
{
  if (type == "Required") {
    return QObject::tr("This component is required");
  } else if (type == "Recommended") {
    return QObject::tr("It is recommended you enable this component");
  } else if (type == "NotUsable") {
    return QObject::tr("This component is not usable in combination with other installed plugins");
  } else if (type == "CouldBeUsable") {
    return QObject::tr("You may be experiencing instability in combination with other installed plugins");
  }
  return QObject::tr("Optional component");
}

}  // namespace

FomodDialog::FomodDialog(QWidget* parent) : QDialog(parent), ui(new Ui::FomodInstallerDialog)
{
  ui->setupUi(this);
  ui->screenshotExpand->setVisible(false);
  ui->descriptionText->setReadOnly(true);
  ui->screenshotLabel->setAlignment(Qt::AlignCenter);
  setWindowModality(Qt::ApplicationModal);
  connect(ui->nextBtn, &QPushButton::clicked, this, [this] {
    answer({{"action", "next"}});
  });
  connect(ui->prevBtn, &QPushButton::clicked, this, [this] {
    answer({{"action", "back"}});
  });
  connect(ui->cancelBtn, &QPushButton::clicked, this, [this] {
    answer({{"action", "cancel"}});
  });
  connect(ui->manualBtn, &QPushButton::clicked, this, [this] {
    answer({{"action", "manual"}});
  });
  connect(ui->websiteLabel, &QLabel::linkActivated, this, [](const QString&) {});
  ui->websiteLabel->setOpenExternalLinks(true);
}

FomodDialog::~FomodDialog()
{
  delete ui;
}

void FomodDialog::answer(QJsonObject a)
{
  if (m_answered) {
    // a second click before Rust answered the first (both in one pass of the event loop)
    // would replace the first answer: it is dropped, as a click on a busy dialog
    return;
  }
  m_answered = true;
  a["name"] = ui->nameCombo->currentText();
  m_answer = a;
  m_loop.quit();
}

void FomodDialog::closeEvent(QCloseEvent* event)
{
  event->ignore();
  answer({{"action", "cancel"}});
}

void FomodDialog::showPlugin(const QString& description, const QString& image)
{
  ui->descriptionText->setPlainText(description);
  QPixmap pixmap;
  if (!image.isEmpty()) {
    pixmap.load(m_images + "/" + QString(image).replace('\\', '/'));
  }
  if (pixmap.isNull()) {
    ui->screenshotLabel->clear();
  } else {
    ui->screenshotLabel->setPixmap(pixmap.scaled(ui->screenshotLabel->size(), Qt::KeepAspectRatio,
                                                 Qt::SmoothTransformation));
  }
}

bool FomodDialog::eventFilter(QObject* object, QEvent* event)
{
  if (event->type() == QEvent::HoverEnter) {
    showPlugin(object->property("description").toString(), object->property("image").toString());
  }
  return QDialog::eventFilter(object, event);
}

void FomodDialog::build(const QJsonObject& r)
{
  // the name and the header come with every request; only fill them the first time
  if (ui->nameCombo->count() == 0) {
    for (const auto& v : r["variants"].toArray()) {
      ui->nameCombo->addItem(v.toString());
    }
    ui->nameCombo->setCurrentText(r["name"].toString());
    const auto info = r["info"].toObject();
    ui->authorLabel->setText(info["author"].toString());
    ui->versionLabel->setText(info["version"].toString());
    const QString site = info["website"].toString();
    ui->websiteLabel->setText(site.isEmpty() ? QString() : tr("<a href=\"%1\">Link</a>").arg(site));
    ui->websiteLabel->setToolTip(site);
    setWindowTitle(info["name"].toString().isEmpty() ? r["name"].toString() : info["name"].toString());
  }
  m_images = r["images"].toString();
  ui->prevBtn->setEnabled(r["can_back"].toBool());

  const QString error = r["error"].toString();
  if (!error.isEmpty()) {
    QMessageBox::information(this, windowTitle(), error);
  }

  const auto step = r["step"].toObject();
  if (r["step"].isNull()) {
    // FomodInstallerDialog::transformToSmallInstall: nothing to choose
    ui->descriptionText->setVisible(false);
    ui->screenshotLabel->setVisible(false);
    ui->stepsStack->setVisible(false);
    m_isLast = true;
    ui->nextBtn->setText(tr("Install"));
    return;
  }

  // keep the description of the hovered plugin while the same step is rebuilt
  const QString key = QString::number(step["index"].toInt());
  const bool sameStep = key == m_stepKey;
  m_stepKey = key;

  while (ui->stepsStack->count() > 0) {
    QWidget* old = ui->stepsStack->widget(0);
    ui->stepsStack->removeWidget(old);
    old->deleteLater();
  }
  auto* page = new QGroupBox(step["name"].toString());
  auto* pageLayout = new QVBoxLayout(page);
  auto* scroll = new QScrollArea;
  auto* inner = new QFrame;
  auto* innerLayout = new QVBoxLayout(inner);
  scroll->setWidget(inner);
  scroll->setWidgetResizable(true);
  pageLayout->addWidget(scroll);

  QString firstDescription, firstImage;
  const auto groups = step["groups"].toArray();
  for (int g = 0; g < groups.size(); ++g) {
    const auto group = groups[g].toObject();
    const QString kind = group["kind"].toString();
    const bool radio = kind == "SelectAtMostOne" || kind == "SelectExactlyOne";
    if (kind == "SelectAtLeastOne") {
      innerLayout->addWidget(new QLabel(tr("Select one or more of these options:")));
    }
    auto* box = new QGroupBox(group["name"].toString());
    auto* boxLayout = new QVBoxLayout(box);
    auto* buttons = new QButtonGroup(box);
    buttons->setExclusive(radio);
    const auto plugins = group["plugins"].toArray();
    bool anyChecked = false;
    for (int p = 0; p < plugins.size(); ++p) {
      const auto plugin = plugins[p].toObject();
      QAbstractButton* control = radio ? static_cast<QAbstractButton*>(new QRadioButton(plugin["name"].toString()))
                                       : new QCheckBox(plugin["name"].toString());
      control->setObjectName("choice");
      control->setChecked(plugin["checked"].toBool());
      control->setEnabled(plugin["enabled"].toBool());
      control->setToolTip(kind == "SelectAll" ? tr("All components in this group are required")
                                              : typeTip(plugin["kind"].toString()));
      control->setProperty("description", plugin["description"].toString());
      control->setProperty("image", plugin["image"].toString());
      control->setAttribute(Qt::WA_Hover);
      control->installEventFilter(this);
      anyChecked = anyChecked || control->isChecked();
      buttons->addButton(control);
      boxLayout->addWidget(control);
      connect(control, &QAbstractButton::clicked, this, [this, g, p](bool on) {
        answer({{"action", "set"}, {"group", g}, {"plugin", p}, {"on", on}});
      });
      if (g == 0 && p == 0) {
        firstDescription = plugin["description"].toString();
        firstImage = plugin["image"].toString();
      }
    }
    if (kind == "SelectAtMostOne") {
      auto* none = new QRadioButton(tr("None"));
      none->setObjectName("none");
      none->setChecked(!anyChecked);
      buttons->addButton(none);
      boxLayout->addWidget(none);
      connect(none, &QAbstractButton::clicked, this, [this, g, plugins] {
        for (int p = 0; p < plugins.size(); ++p) {
          if (plugins[p].toObject()["checked"].toBool()) {
            answer({{"action", "set"}, {"group", g}, {"plugin", p}, {"on", false}});
            return;
          }
        }
      });
    }
    innerLayout->addWidget(box);
  }
  innerLayout->addStretch();
  ui->stepsStack->addWidget(page);
  if (!sameStep) {
    showPlugin(firstDescription, firstImage);
  }

  // updateNextbtnText
  QStringList blocking;
  for (const auto& b : step["blocking"].toArray()) {
    blocking << b.toString();
  }
  m_isLast = step["is_last"].toBool();
  if (!blocking.isEmpty()) {
    ui->nextBtn->setText(tr("Disabled"));
    ui->nextBtn->setEnabled(false);
    ui->nextBtn->setToolTip(tr("This button is disabled because the following group(s) need a selection: ") +
                            blocking.join(", "));
  } else {
    ui->nextBtn->setEnabled(true);
    ui->nextBtn->setToolTip(QString());
    ui->nextBtn->setText(m_isLast ? tr("Install") : tr("Next"));
  }
}

QJsonObject FomodDialog::wait(const QJsonObject& request)
{
  build(request);
  if (!isVisible()) {
    show();
  }
  m_answer   = {{"action", "cancel"}};
  m_answered = false;
  m_loop.exec();
  const QString action = m_answer["action"].toString();
  if (action == "cancel" || action == "manual" || (action == "next" && m_isLast)) {
    hide();
  }
  return m_answer;
}

}  // namespace strata

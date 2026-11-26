// MO2's GeometrySaver (settings.h): a dialog's size and place restored when it opens and
// stored when it closes, under [Geometry] <objectName>_geometry in the instance's
// ModOrganizer.ini (GeometrySettings::saveWindowGeometry, geoSettingName), so MO2 and
// Strata restore each other's dialogs.
#pragma once

#include <QPointer>
#include <QString>

class QDialog;

class GeometrySaver
{
public:
  // `root`: the instance folder ("" does nothing).
  GeometrySaver(const QString& root, QDialog* dialog);
  ~GeometrySaver();
  GeometrySaver(const GeometrySaver&)            = delete;
  GeometrySaver& operator=(const GeometrySaver&) = delete;

private:
  QString m_root;
  QPointer<QDialog> m_dialog;
};

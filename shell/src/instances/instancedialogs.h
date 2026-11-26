#pragma once

// The instance manager (MO2's InstanceManagerDialog, forms/instancemanagerdialog.ui) and
// the new-instance wizard (MO2's CreateInstanceDialog, rebuilt for Proton). The logic is
// strata_ui::instances (crates/strata-ffi/src/bridge_instances.rs).

#include <QString>

class QWidget;

namespace strata::instances_ui
{

// Shows the instance manager. `current` is the open instance's folder ("" when none).
// Returns the folder the user chose to switch to, or "" (closed).
QString showInstanceManager(QWidget* parent, const QString& current);

// Shows the wizard alone (the manager's "Create new instance"). Returns the new instance's
// folder when the user asked to switch to it, else "". `created` gets the folder of an
// instance that was made either way.
QString showCreateInstance(QWidget* parent, QString* created = nullptr);

}  // namespace strata::instances_ui

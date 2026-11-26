# The shell: views, delegates, dialogs. It owns no state (spec/06-ui.md).

set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTOUIC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_AUTOUIC_SEARCH_PATHS ${CMAKE_CURRENT_SOURCE_DIR}/forms ${CMAKE_CURRENT_SOURCE_DIR}/forms/install)

# Everything but main() is one object library: the window (strata) and the test program
# that drives it (strata-shell-test, tests/) are built from the same objects.
qt_add_library(strata_shell OBJECT
  src/mainwindow.cpp
  src/mainwindow.h
  src/columnfit.cpp
  src/columnfit.h
  src/datatab.cpp
  src/filterpanel.cpp
  src/filterpanel.h
  src/pluginstab.cpp
  src/pluginstab.h
  src/datatab.h
  src/downloadproxy.h
  src/geometrysaver.cpp
  src/geometrysaver.h
  src/overwritedialog.cpp
  src/overwritedialog.h
  forms/overwriteinfodialog.ui
  src/layoutstate.cpp
  src/layoutstate.h
  src/loglist.cpp
  src/modlist.cpp
  src/modlist.h
  src/style.cpp
  src/style.h
  src/widgets/linklabel.h
  src/widgets/lineeditclear.h
  src/widgets/modlistview.h
  src/widgets/pluginlistview.h
  src/widgets/lcdnumber.h
  src/widgets/sortabletreewidget.h
  src/widgets/loglist.h
  src/widgets/columnmenu.h
  src/widgets/statusbar.h
  src/widgets/downloadlistview.h
  forms/mainwindow.ui
  # conflicts area: mod info dialog, previews
  src/conflicts/imagestabhelpers.h
  src/conflicts/modinfodialog.cpp
  src/conflicts/modinfodialog.h
  src/conflicts/preview.cpp
  src/conflicts/preview.h
  src/conflicts/texteditor.h
  forms/modinfodialog.ui
  forms/previewdialog.ui
  # install area: installer dialogs
  src/install/installhost.cpp
  src/install/installhost.h
  src/install/fomoddialog.cpp
  src/install/fomoddialog.h
  src/install/downloadactions.cpp
  src/install/downloadactions.h
  src/install/downloadqueue.cpp
  src/install/downloadqueue.h
  forms/install/simpleinstalldialog.ui
  forms/install/baincomplexinstallerdialog.ui
  forms/install/multiarchivedialog.ui
  forms/install/installdialog.ui
  forms/install/fomodinstallerdialog.ui
  forms/install/queryoverwritedialog.ui
  # the LOOT dialog (Sort button)
  src/loot/loothost_api.h
  src/loot/lootdialog.cpp
  src/loot/lootdialog.h
  src/loot/lootreport.cpp
  src/loot/lootreport.h
  forms/lootdialog.ui
  resources.qrc
  stylesheet_resource.qrc
)

# Area "modlist" (round 2): the profiles, executables and settings dialogs, the delete
# confirmation (shell/src/modlist/, bridge crates/strata-ffi/src/bridge_modlist.rs).
target_sources(strata_shell PRIVATE
  src/modlist/colortable.h
  src/modlist/common.h
  src/modlist/crashlogviewer.cpp
  src/modlist/crashlogviewer.h
  src/modlist/deleteconfirm.cpp
  src/modlist/deleteconfirm.h
  src/modlist/dialogs.cpp
  src/modlist/dialogs.h
  src/modlist/executablesdialog.cpp
  src/modlist/executablesdialog.h
  src/modlist/inieditor.cpp
  src/modlist/inieditor.h
  src/modlist/problemsdialog.cpp
  src/modlist/problemsdialog.h
  src/modlist/profilesdialog.cpp
  src/modlist/profilesdialog.h
  src/modlist/settingsdialog.cpp
  src/modlist/settingsdialog.h
  forms/editexecutablesdialog.ui
  forms/profileinputdialog.ui
  forms/profilesdialog.ui
  forms/settingsdialog.ui
  forms/nexusmanualkey.ui
  forms/problemsdialog.ui
  forms/textviewer.ui
)

# The instance manager and the new-instance wizard (bridge_instances.rs).
target_sources(strata_shell PRIVATE
  src/instances/instancedialogs.cpp
  src/instances/instancedialogs.h
  forms/instancemanagerdialog.ui
  src/listtools.cpp
  src/listtools.h
  forms/savetextasdialog.ui
  forms/syncoverwritedialog.ui
)

target_include_directories(strata_shell PUBLIC src src/widgets src/modlist)
target_compile_options(strata_shell PRIVATE -Wall -Wextra)
target_link_libraries(strata_shell PUBLIC Qt6::Widgets Qt6::Network strata_bridge)

qt_add_executable(strata src/main.cpp)
target_link_libraries(strata PRIVATE strata_shell)

# build/strata, with MO2's stylesheets next to it.
set_target_properties(strata PROPERTIES RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR})
add_custom_command(TARGET strata POST_BUILD
  COMMAND ${CMAKE_COMMAND} -E copy_directory_if_different
          ${CMAKE_CURRENT_SOURCE_DIR}/stylesheets ${CMAKE_BINARY_DIR}/stylesheets
  VERBATIM)

if(STRATA_BUILD_TESTS)
# install area: shows one installer dialog offscreen and saves a picture of it
# (tools/shell-test-install.sh): strata-dialog-shot <kind> <request.json> <out.png>
qt_add_executable(strata-dialog-shot
  src/install/dialogshot.cpp
  src/install/downloadqueue.cpp
  src/install/downloadqueue.h
  src/install/installhost.cpp
  src/install/installhost.h
  src/install/fomoddialog.cpp
  src/install/fomoddialog.h
  forms/install/simpleinstalldialog.ui
  forms/install/baincomplexinstallerdialog.ui
  forms/install/multiarchivedialog.ui
  forms/install/installdialog.ui
  forms/install/fomodinstallerdialog.ui
  forms/install/queryoverwritedialog.ui
)
target_include_directories(strata-dialog-shot PRIVATE src)
target_link_libraries(strata-dialog-shot PRIVATE Qt6::Widgets strata_bridge)
set_target_properties(strata-dialog-shot PROPERTIES RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR})

# shell area (round 3): drives the real window offscreen with Qt Test (clicks, keys, drops,
# context menus, dialogs answered) on a generated instance: tools/shell-drive.sh.
find_package(Qt6 6.5 REQUIRED COMPONENTS Test)
qt_add_executable(strata-shell-test
  tests/drive.cpp
  tests/driver.h
  tests/tests.h
  tests/mainwindow_test.cpp
  tests/dialogs_test.cpp
  tests/drops_test.cpp
  tests/data_test.cpp
  tests/overwrite_test.cpp
  tests/menus_test.cpp
  tests/tools_test.cpp
  tests/tour_test.cpp
  tests/columns_test.cpp
  tests/loot_test.cpp
  tests/instances_test.cpp
  tests/listtools_test.cpp
)
target_compile_options(strata-shell-test PRIVATE -Wall -Wextra)
target_link_libraries(strata-shell-test PRIVATE strata_shell Qt6::Test)
set_target_properties(strata-shell-test PROPERTIES RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR})
endif()  # STRATA_BUILD_TESTS

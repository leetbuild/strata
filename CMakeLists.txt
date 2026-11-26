cmake_minimum_required(VERSION 3.24)
project(strata VERSION 0.1.0 LANGUAGES CXX)

# One binary: the C++ Qt shell plus the Rust crates as one static library.
# See docs/ARCHITECTURE.md, "Entry point". Build with tools/build.sh.

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

find_package(Qt6 6.5 REQUIRED COMPONENTS Widgets Network)
find_package(Corrosion REQUIRED)
include(GNUInstallDirs)

# The test programs (strata-shell-test, strata-dialog-shot): off for packages.
option(STRATA_BUILD_TESTS "Build the window's test programs" ON)

# Rust: strata-ffi is the static library; it pulls in every other crate.
corrosion_import_crate(MANIFEST_PATH Cargo.toml CRATES strata-ffi CRATE_TYPES staticlib)
# The cxx bridge: generates strata_bridge/bridge.h from crates/strata-ffi/src/bridge.rs.
# One file per area (round 2); bridge_install.rs names a C++ type of the shell.
corrosion_add_cxxbridge(strata_bridge CRATE strata_ffi FILES
  bridge.rs
  bridge_modlist.rs    # modlist area
  bridge_builtins.rs   # modlist area, round 3
  bridge_crashlogs.rs  # Crash Log Viewer / Labeler
  bridge_conflicts.rs  # conflicts area
  bridge_install.rs    # install area
  bridge_loot.rs       # the LOOT dialog
  bridge_instances.rs  # the instance manager and the new-instance wizard
)
# bridge_install.rs and bridge_loot.rs name C++ types of the shell
target_include_directories(strata_bridge PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/shell/src)

add_subdirectory(shell)

# `cmake --install`: the program, MO2's stylesheets, the desktop entry and the icons.
install(TARGETS strata RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
install(DIRECTORY shell/stylesheets/ DESTINATION ${CMAKE_INSTALL_DATADIR}/strata/stylesheets)
install(FILES data/strata.desktop DESTINATION ${CMAKE_INSTALL_DATADIR}/applications)
foreach(size 32 48 64 128 256 512)
  install(FILES data/icons/hicolor/${size}x${size}/strata.png
          DESTINATION ${CMAKE_INSTALL_DATADIR}/icons/hicolor/${size}x${size}/apps)
endforeach()

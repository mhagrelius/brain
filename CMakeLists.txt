cmake_minimum_required(VERSION 3.21)
project(brain VERSION 0.2.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Qt6 6.5 REQUIRED COMPONENTS Core Gui Qml Quick QuickControls2 DBus Svg Network Test)
qt_standard_project_setup(REQUIRES 6.5)
qt_policy(SET QTP0004 NEW)  # .qml files in subdirectories without a qmldir

# ---- core: the vault, notes, index, search, vectors and sync (QtCore only) ---
qt_add_library(braincore STATIC
    src/core/scanner.cpp
    src/core/frontmatter.cpp
    src/core/note.cpp
    src/core/vault.cpp
    src/core/index.cpp
    src/core/search.cpp
    src/core/semantic.cpp
    src/core/sync.cpp
    src/core/tree.cpp
    src/core/config.cpp
    src/core/jsonutil.cpp
    src/core/notebook.cpp
)
target_include_directories(braincore PUBLIC src/core)
target_link_libraries(braincore PUBLIC Qt6::Core)

# ---- net: the sockets, on worker threads (QtNetwork) -------------------------
qt_add_library(brainnet STATIC
    src/net/http.cpp
    src/net/embedder.cpp
    src/net/vaultserver.cpp
)
target_include_directories(brainnet PUBLIC src/net)
target_link_libraries(brainnet PUBLIC braincore Qt6::Network)

# ---- app: the App singleton, the editor and the QML views --------------------
qt_add_executable(brain
    src/main.cpp
    src/systemtheme.cpp
    src/palette.cpp
    src/backend.cpp
    src/editor.cpp
    src/icons.cpp
    src/single.cpp
)
file(GLOB BRAIN_ICONS RELATIVE ${CMAKE_SOURCE_DIR} CONFIGURE_DEPENDS src/icons/*.svg)

set_source_files_properties(src/qml/T.qml PROPERTIES QT_QML_SINGLETON_TYPE TRUE)

qt_add_qml_module(brain
    URI Brain
    VERSION 1.0
    SOURCES
        src/palette.h
        src/backend.h
        src/editor.h
    QML_FILES
        src/qml/Main.qml
        src/qml/T.qml
        src/qml/components/Card.qml
        src/qml/components/Eyebrow.qml
        src/qml/components/Field.qml
        src/qml/components/HoverRow.qml
        src/qml/components/Mono.qml
        src/qml/components/PrimaryButton.qml
        src/qml/components/Sans.qml
        src/qml/components/SecondaryButton.qml
        src/qml/components/TipArea.qml
        src/qml/components/Tooltip.qml
        src/qml/components/VScroll.qml
        src/qml/components/Icon.qml
        src/qml/components/Keycap.qml
        src/qml/components/Popover.qml
        src/qml/screens/Sidebar.qml
        src/qml/screens/NotePane.qml
        src/qml/screens/Rail.qml
        src/qml/screens/CapturePanel.qml
        src/qml/screens/InsertMenu.qml
        src/qml/screens/SearchPanel.qml
        src/qml/screens/Toasts.qml
        src/qml/screens/PromptPopup.qml
        src/qml/screens/StatusPanel.qml
        src/qml/screens/FirstRun.qml
        src/qml/screens/RowMenu.qml
        src/qml/screens/EditorMenu.qml
    RESOURCES ${BRAIN_ICONS}
)

target_include_directories(brain PRIVATE src)
target_link_libraries(brain PRIVATE braincore brainnet
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::DBus Qt6::Svg Qt6::Network)

# ---- tests: one QtTest executable per core area ------------------------------
enable_testing()
foreach(t scanner frontmatter vault index search semantic sync notebook wire)
    qt_add_executable(test_${t} tests/test_${t}.cpp)
    target_link_libraries(test_${t} PRIVATE braincore brainnet Qt6::Test)
    add_test(NAME ${t} COMMAND test_${t})
endforeach()

include(GNUInstallDirs)
install(TARGETS brain RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
install(FILES packaging/brain.desktop DESTINATION ${CMAKE_INSTALL_DATADIR}/applications)
install(FILES packaging/brain.svg DESTINATION ${CMAKE_INSTALL_DATADIR}/icons/hicolor/scalable/apps)

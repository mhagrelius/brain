#pragma once

// The app model, exposed to QML as the `App` singleton. Holds the Notebook
// and every derived figure; QML is a view over pre-formatted rows. One
// `changed` signal after recompute() so screens can never disagree.

#include "editor.h"
#include "notebook.h"

#include <QFileSystemWatcher>
#include <QObject>
#include <QQmlEngine>
#include <QStringList>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>

#include <functional>

class Palette;

class Backend : public QObject {
    Q_OBJECT
    QML_NAMED_ELEMENT(App)
    QML_SINGLETON

    Q_PROPERTY(QString screen READ screen WRITE setScreen NOTIFY changed)
    Q_PROPERTY(QVariantList nav READ nav NOTIFY changed)
    Q_PROPERTY(QString vaultLabel READ vaultLabel NOTIFY changed)
    Q_PROPERTY(bool hasVault READ hasVault NOTIFY changed)
    Q_PROPERTY(QString vaultRoot READ vaultRoot NOTIFY changed)
    Q_PROPERTY(QVariantList rows READ rows NOTIFY changed)
    Q_PROPERTY(QVariantList tagRows READ tagRows NOTIFY changed)
    Q_PROPERTY(QString activeTag READ activeTag NOTIFY changed)
    Q_PROPERTY(QString sortLabel READ sortLabel NOTIFY changed)
    Q_PROPERTY(QString query READ query NOTIFY changed)
    Q_PROPERTY(QString sidebarLabel READ sidebarLabel NOTIFY changed)

    Q_PROPERTY(bool hasNote READ hasNote NOTIFY changed)
    Q_PROPERTY(QString noteId READ noteId NOTIFY changed)
    Q_PROPERTY(QString noteTitle READ noteTitle NOTIFY changed)
    Q_PROPERTY(QString noteSubtitle READ noteSubtitle NOTIFY changed)
    Q_PROPERTY(QVariantMap rail READ rail NOTIFY changed)
    Q_PROPERTY(QVariantList backlinks READ backlinks NOTIFY changed)
    Q_PROPERTY(QString statusLeft READ statusLeft NOTIFY changed)
    Q_PROPERTY(QString statusRight READ statusRight NOTIFY changed)
    Q_PROPERTY(QString statusDot READ statusDot NOTIFY changed)
    Q_PROPERTY(bool reading READ reading NOTIFY changed)
    Q_PROPERTY(Editor *editor READ editor CONSTANT)

    Q_PROPERTY(bool captureOpen READ captureOpen NOTIFY changed)
    Q_PROPERTY(QString captureDestination READ captureDestination NOTIFY changed)
    Q_PROPERTY(bool insertOpen READ insertOpen NOTIFY changed)
    Q_PROPERTY(QVariantList insertRows READ insertRows NOTIFY changed)
    Q_PROPERTY(QVariantList recentRows READ recentRows NOTIFY changed)
    Q_PROPERTY(QString insertFilterText READ insertFilterText NOTIFY changed)
    Q_PROPERTY(bool searchOpen READ searchOpen NOTIFY changed)
    Q_PROPERTY(QString searchMode READ searchMode NOTIFY changed)
    Q_PROPERTY(QString searchQuery READ searchQuery NOTIFY changed)
    Q_PROPERTY(QVariantList hits READ hits NOTIFY changed)
    Q_PROPERTY(QString searchRanking READ searchRanking NOTIFY changed)
    Q_PROPERTY(QString createDestination READ createDestination NOTIFY changed)
    Q_PROPERTY(QVariantList toasts READ toasts NOTIFY changed)
    Q_PROPERTY(QVariantMap prompt READ prompt NOTIFY changed)
    Q_PROPERTY(QVariantMap statusInfo READ statusInfo NOTIFY changed)
    Q_PROPERTY(bool statusOpen READ statusOpen NOTIFY changed)

public:
    explicit Backend(Palette *palette, QObject *parent = nullptr);
    ~Backend() override;
    static Backend *create(QQmlEngine *, QJSEngine *);
    static Backend *instance() { return s_instance; }

    QString screen() const { return m_screen; }
    void setScreen(const QString &screen);
    QVariantList nav() const { return m_nav; }
    QString vaultLabel() const { return m_vaultLabel; }
    bool hasVault() const { return m_notebook.hasVault(); }
    QString vaultRoot() const { return m_notebook.vaultRoot(); }
    QVariantList rows() const { return m_rows; }
    QVariantList tagRows() const { return m_tagRows; }
    QString activeTag() const { return m_notebook.activeTag().value_or(QString()); }
    QString sortLabel() const;
    QString query() const { return m_notebook.query(); }
    QString sidebarLabel() const { return m_sidebarLabel; }

    bool hasNote() const { return m_notebook.openNoteId().has_value(); }
    QString noteId() const { return m_notebook.openNoteId() ? m_notebook.openNoteId()->str() : QString(); }
    QString noteTitle() const { return m_noteTitle; }
    QString noteSubtitle() const { return m_noteSubtitle; }
    QVariantMap rail() const { return m_rail; }
    QVariantList backlinks() const { return m_backlinks; }
    QString statusLeft() const { return m_statusLeft; }
    QString statusRight() const { return m_statusRight; }
    QString statusDot() const { return m_statusDot; }
    bool reading() const { return m_reading; }
    Editor *editor() const { return m_editor; }

    bool captureOpen() const { return m_captureOpen; }
    QString captureDestination() const;
    bool insertOpen() const { return m_insertOpen; }
    QVariantList insertRows() const { return m_insertRows; }
    QVariantList recentRows() const { return m_recentRows; }
    QString insertFilterText() const { return m_insertQuery; }
    bool searchOpen() const { return m_searchOpen; }
    QString searchMode() const { return m_searchMode == brain::Notebook::Mode::Title ? QStringLiteral("titles") : QStringLiteral("text"); }
    QString searchQuery() const { return m_searchQuery; }
    QVariantList hits() const { return m_hits; }
    QString searchRanking() const { return m_searchRanking; }
    QString createDestination() const;
    QVariantList toasts() const { return m_toasts; }
    QVariantMap prompt() const { return m_prompt; }
    QVariantMap statusInfo() const { return m_statusInfo; }
    bool statusOpen() const { return m_statusOpen; }

    // ---- navigation and the sidebar ----
    Q_INVOKABLE void go(const QString &screen) { setScreen(screen); }
    Q_INVOKABLE void goToKey(int key);
    Q_INVOKABLE void act(const QString &name);
    Q_INVOKABLE void openNote(const QString &id);
    Q_INVOKABLE void toggleFolder(const QString &path);
    Q_INVOKABLE void setQuery(const QString &text);
    Q_INVOKABLE void cycleSort();
    Q_INVOKABLE void filterTag(const QString &tag);
    Q_INVOKABLE void chooseVault(const QString &path);
    Q_INVOKABLE QString suggestedVault() const;

    // ---- notes and folders ----
    Q_INVOKABLE void newNote();
    Q_INVOKABLE void newNoteIn(const QString &folder);
    Q_INVOKABLE void newFolderIn(const QString &parent);
    Q_INVOKABLE void askRenameNote(const QString &id);
    Q_INVOKABLE void askDeleteNote(const QString &id);
    Q_INVOKABLE void askRenameFolder(const QString &path);
    Q_INVOKABLE void askDeleteFolder(const QString &path);
    Q_INVOKABLE void moveNote(const QString &id, const QString &folder);
    Q_INVOKABLE void moveFolder(const QString &from, const QString &into);
    Q_INVOKABLE void promptAccept(const QString &value);
    Q_INVOKABLE void promptCancel();

    // ---- the editor ----
    Q_INVOKABLE void toggleReading();
    Q_INVOKABLE void saveNow();
    Q_INVOKABLE void followLink(const QString &target);
    Q_INVOKABLE void openUrl(const QString &url);
    Q_INVOKABLE QStringList linkCandidates(const QString &query) const { return m_notebook.linkCandidates(query); }
    Q_INVOKABLE void attachUrls(const QStringList &urls);
    Q_INVOKABLE bool pasteImage();
    Q_INVOKABLE void setNoteTags(const QString &text);
    Q_INVOKABLE void setNoteAliases(const QString &text);

    // ---- summoned surfaces ----
    Q_INVOKABLE void openCapture();
    Q_INVOKABLE void closeCapture();
    Q_INVOKABLE void captureCycleDestination();
    Q_INVOKABLE void capture(const QString &text, bool openAfter);
    Q_INVOKABLE void openInsert();
    Q_INVOKABLE void closeInsert();
    Q_INVOKABLE void insertFilter(const QString &text);
    Q_INVOKABLE void applyInsert(const QString &label, int selectionStart, int selectionEnd);
    Q_INVOKABLE void openSearch(const QString &mode);
    Q_INVOKABLE void closeSearch();
    Q_INVOKABLE void searchToggleMode();
    Q_INVOKABLE void setSearchQuery(const QString &text);
    Q_INVOKABLE void openHit(const QString &id);
    Q_INVOKABLE void createFromSearch();
    Q_INVOKABLE void toastAction(int id, const QString &which);
    Q_INVOKABLE void dismissToast(int id);
    Q_INVOKABLE void toggleStatus();
    Q_INVOKABLE void syncNow();

    // CLI verbs forwarded from a second launch.
    QString handleVerb(const QStringList &args);
    void flushAndSave();

signals:
    void changed();
    void windowRequested();
    void captureRequested();
    void searchFocusRequested();
    void editorFocusRequested();

private:
    enum class ToastKind { Success, Conflict, Vanished, Osd, Info };
    void recompute();
    void refreshTree();
    void refreshHits();
    void refreshInsert();
    void refreshStatusInfo();
    void loadOpenNoteIntoEditor(int caret = 0);
    void flushEditor();
    void onTick();
    void onEdited();
    void watchVault();
    void onVaultChanged();
    void scheduleCatchUp(int delayMs);
    void runCatchUp();
    void runSync();
    void runQueryEmbedding(const QString &query);
    static void runOnMain(std::function<void()> fn);
    int toast(ToastKind kind, const QString &title, const QString &body, const QString &primary = QString(), const QString &secondary = QString(), int ttlMs = 4000);
    void removeToast(int id);
    void setOsd(const QString &key, const QString &title, const QString &detail, double progress, int ttlMs);
    void openPrompt(const QString &kind, const QString &title, const QString &value, const QString &primary, const QString &target, bool danger = false);
    QString relative(qint64 seconds) const;
    void refreshToastsSoon();

    static Backend *s_instance;
    Palette *m_palette;
    brain::Notebook m_notebook;
    Editor *m_editor;
    QString m_screen = QStringLiteral("notes");
    bool m_reading = false;
    QFileSystemWatcher m_watcher;
    QTimer m_tick, m_watchDebounce, m_catchUpTimer, m_syncTimer, m_toastTimer, m_queryTimer;
    bool m_catchingUp = false, m_syncing = false, m_catchUpAgain = false;
    int m_generation = 0;
    QString m_pendingQuery;
    QStringList m_recent;
    QString m_insertQuery;

    QVariantList m_nav, m_rows, m_tagRows, m_backlinks, m_insertRows, m_recentRows, m_hits, m_toasts;
    QVariantMap m_rail, m_prompt, m_statusInfo;
    QString m_vaultLabel, m_sidebarLabel, m_noteTitle, m_noteSubtitle, m_statusLeft, m_statusRight, m_statusDot, m_searchRanking;
    bool m_captureOpen = false, m_insertOpen = false, m_searchOpen = false, m_statusOpen = false;
    int m_captureTarget = 0;   // 0 = Inbox.md, 1 = the open note
    brain::Notebook::Mode m_searchMode = brain::Notebook::Mode::Title;
    QString m_searchQuery;
    int m_nextToast = 1;
    QString m_embeddingLabel;
    std::pair<int, int> m_embedding{0, 0};
    QString m_syncedAt;
    QString m_machine;
    QString m_lastEmbeddingError;
};

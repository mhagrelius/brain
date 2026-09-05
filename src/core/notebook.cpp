#include "notebook.h"

#include <QDir>
#include <QFileInfo>

namespace brain {

namespace {
QString join(const QString &parent, const QString &name) { return parent.isEmpty() ? name : parent + u'/' + name; }
}

// ---- config ---------------------------------------------------------------

void Notebook::loadConfig(const QString &path) {
    m_configPath = path;
    m_config = Config::load(path).first;
    m_sort = tree::sortFromName(m_config.sort);
    for (const QString &folder : m_config.expandedFolders) m_expanded.insert(folder);
}

bool Notebook::saveConfig(QString *error) {
    m_config.sort = tree::sortName(m_sort);
    QStringList expanded(m_expanded.begin(), m_expanded.end());
    expanded.sort();
    m_config.expandedFolders = expanded;
    return m_config.save(m_configPath, error);
}

std::optional<QString> Notebook::embeddingUrl() const {
    if (!m_config.embeddingUrl) return QString::fromLatin1(DEFAULT_EMBEDDING_URL);
    const QString url = m_config.embeddingUrl->trimmed();
    if (url.isEmpty()) return std::nullopt;
    return url;
}

namespace {
// Both or neither: a URL with no token cannot authenticate. An absent URL
// means the default server; an empty one means off.
std::optional<std::pair<QString, QString>> service(const std::optional<QString> &url, const std::optional<QString> &token) {
    const QString u = url ? url->trimmed() : QString::fromLatin1(DEFAULT_SERVER_URL);
    const QString t = token ? token->trimmed() : QString();
    if (u.isEmpty() || t.isEmpty()) return std::nullopt;
    QString clean = u;
    while (clean.endsWith(u'/')) clean.chop(1);
    return std::make_pair(clean, t);
}
}

std::optional<std::pair<QString, QString>> Notebook::sharedVectors() const { return service(m_config.vectorsUrl, m_config.vectorsToken); }
std::optional<std::pair<QString, QString>> Notebook::syncServer() const { return service(m_config.syncUrl, m_config.syncToken); }

// ---- vault ----------------------------------------------------------------

QVector<VaultError> Notebook::setVault(const QString &root) {
    m_open.reset();
    m_buffer.reset();
    m_onDisk.reset();
    m_dirty = false;
    clearAlerts();
    m_vault = Vault(root);
    m_config.vault = m_vault.root();
    m_config.lastNote.reset();
    loadVectors();
    return rescan();
}

QVector<VaultError> Notebook::rescan() {
    QVector<VaultError> problems;
    if (m_vault.isNull()) {
        m_index = Index();
        m_lexical = search::Bm25();
        return problems;
    }
    const QVector<Note> notes = m_vault.scan(&problems);
    m_index = Index::build(notes);
    m_lexical = search::Bm25::build(m_index);
    return problems;
}

QVector<VaultError> Notebook::reloadVault() {
    const QVector<VaultError> problems = rescan();
    if (m_open && m_index.contains(*m_open)) {
        loadNote(*m_open);
    } else {
        m_open.reset();
        m_buffer.reset();
        m_onDisk.reset();
    }
    return problems;
}

void Notebook::loadVectors() {
    if (m_vault.isNull()) { m_vectors = semantic::Store(); m_vectorsPath.reset(); return; }
    const QString path = semantic::defaultStorePath(m_vault.root());
    m_vectors = semantic::Store::load(path);
    m_vectorsPath = path;
}

// ---- alerts ---------------------------------------------------------------

Alert Notebook::alert() const {
    Alert a;
    if (m_notSaving) { a.kind = Alert::NotSaving; a.message = *m_notSaving; return a; }
    if (m_diverged) { a.kind = Alert::Diverged; a.id = *m_diverged; return a; }
    if (m_vanished) { a.kind = Alert::Vanished; a.id = *m_vanished; return a; }
    if (m_conflicts > 0) { a.kind = Alert::Conflicts; a.count = m_conflicts; }
    return a;
}

void Notebook::clearAlerts() {
    m_notSaving.reset();
    m_diverged.reset();
    m_vanished.reset();
}

External Notebook::absorbExternalChanges() {
    rescan();
    External e;
    if (!m_open) return e;
    const NoteId id = *m_open;
    if (!m_index.contains(id)) { m_vanished = id; e.kind = External::Vanished; e.id = id; return e; }
    m_vanished.reset();
    const auto note = m_vault.read(id);
    if (!note) return e;
    const QString text = note->toText();
    // An event is not a change: the watcher fires for our own saves too.
    if (m_onDisk && *m_onDisk == text) return e;
    m_onDisk = text;
    if (m_dirty) { m_diverged = id; e.kind = External::Diverged; e.id = id; e.onDisk = text; return e; }
    m_buffer = note;
    m_diverged.reset();
    e.kind = External::Reloaded;
    e.id = id;
    return e;
}

bool Notebook::takeDiskVersion() {
    if (!m_open || m_vault.isNull()) return false;
    const auto note = m_vault.read(*m_open);
    if (!note) return false;
    m_onDisk = note->toText();
    m_buffer = note;
    m_dirty = false;
    m_diverged.reset();
    return true;
}

bool Notebook::restoreOpenNote() {
    if (m_vault.isNull() || !m_buffer) return false;
    if (!m_vault.write(*m_buffer)) return false;
    m_onDisk = m_buffer->toText();
    m_index.update(*m_buffer);
    m_lexical = search::Bm25::build(m_index);
    m_dirty = false;
    m_vanished.reset();
    return true;
}

// ---- vectors --------------------------------------------------------------

void Notebook::absorbVectors(const semantic::Store &store) {
    m_vectors = store;
    if (m_vectorsPath) m_vectors.save(*m_vectorsPath);
}

// ---- listing --------------------------------------------------------------

QVector<NoteId> Notebook::taggedIds() const {
    if (m_filter) return m_index.notesTagged(*m_filter);
    return m_index.ids();
}

QVector<std::pair<NoteId, QString>> Notebook::listedNotes() const {
    if (isSearching()) return searchResults();
    QVector<std::pair<NoteId, QString>> notes;
    for (const NoteId &id : taggedIds()) notes.append({id, m_index.excerpt(id)});
    std::sort(notes.begin(), notes.end(), [](const auto &a, const auto &b) { return a.first < b.first; });
    return notes;
}

QVector<std::pair<NoteId, QString>> Notebook::searchResults() const {
    const QString query = m_query.trimmed();
    if (query.isEmpty()) return {};
    QSet<NoteId> allowed;
    for (const NoteId &id : taggedIds()) allowed.insert(id);
    QVector<std::pair<NoteId, QString>> out;
    for (const search::TitleMatch &m : search::byTitle(m_index, query, 50))
        if (allowed.contains(m.id)) out.append({m.id, m_index.excerpt(m.id)});
    for (const search::TextMatch &m : search::byText(m_index, query, 50)) {
        if (!allowed.contains(m.id)) continue;
        bool have = false;
        for (const auto &o : out) if (o.first == m.id) { have = true; break; }
        if (have) continue;
        out.append({m.id, m.snippets.isEmpty() ? QString() : m.snippets.first().text});
    }
    return out;
}

QVector<tree::Row> Notebook::sidebarRows() const {
    QVector<tree::Listed> notes;
    for (const NoteId &id : taggedIds()) {
        tree::Listed listed;
        listed.id = id;
        listed.excerpt = m_index.excerpt(id);
        if (!m_vault.isNull()) {
            // The sidebar shows a relative mtime on every row, so times are
            // read whichever sort is on.
            const auto [modified, created] = m_vault.times(id);
            listed.modified = modified;
            listed.created = created;
        }
        notes.append(listed);
    }
    const bool filtered = m_filter.has_value();
    const QStringList folders = (!filtered && !m_vault.isNull()) ? m_vault.folders() : QStringList();
    QSet<QString> expanded;
    if (filtered) {
        for (const tree::Listed &note : notes)
            for (const QString &ancestor : tree::ancestors(note.id.folder())) expanded.insert(ancestor);
    } else {
        expanded = m_expanded;
    }
    return tree::rows(notes, folders, expanded, m_sort);
}

bool Notebook::filterByTag(const std::optional<QString> &tag) {
    std::optional<QString> wanted;
    if (tag) {
        QString t = *tag;
        while (t.startsWith(u'#')) t.remove(0, 1);
        wanted = t.toLower();
    }
    const bool known = wanted && !m_index.notesTagged(*wanted).isEmpty();
    m_filter = known ? wanted : std::nullopt;
    return known || !tag;
}

void Notebook::toggleFolder(const QString &path) {
    if (!m_expanded.remove(path)) m_expanded.insert(path);
    m_targetFolder = path;
}

void Notebook::expandTo(const NoteId &id) {
    for (const QString &ancestor : tree::ancestors(id.folder())) m_expanded.insert(ancestor);
}

bool Notebook::setQuery(const QString &query) {
    if (m_query == query) return false;
    m_query = query;
    return true;
}

// ---- the open note --------------------------------------------------------

bool Notebook::loadNote(const NoteId &id, VaultError *error) {
    if (m_vault.isNull()) return false;
    clearAlerts();
    const auto note = m_vault.read(id, error);
    if (!note) {
        m_open.reset();
        m_buffer.reset();
        m_onDisk.reset();
        return false;
    }
    m_onDisk = note->toText();
    m_open = id;
    m_buffer = note;
    m_dirty = false;
    m_config.lastNote = id.str();
    m_targetFolder = id.folder();
    expandTo(id);
    return true;
}

std::optional<NoteId> Notebook::restoreLastNote() {
    if (!m_config.lastNote) return std::nullopt;
    const NoteId id = NoteId::fromRelative(*m_config.lastNote);
    if (!m_index.contains(id)) return std::nullopt;
    if (!loadNote(id)) return std::nullopt;
    return id;
}

void Notebook::flushBody(const QString &body) {
    if (!m_buffer) return;
    if (m_buffer->body == body) return;
    m_buffer->body = body;
    m_dirty = true;
}

void Notebook::setOpenTags(const QStringList &tags) {
    if (!m_buffer) return;
    if (!m_buffer->frontmatter) m_buffer->frontmatter = Frontmatter();
    if (m_buffer->frontmatter->tags == tags) return;
    m_buffer->frontmatter->tags = tags;
    m_dirty = true;
}

void Notebook::setOpenAliases(const QStringList &aliases) {
    if (!m_buffer) return;
    if (!m_buffer->frontmatter) m_buffer->frontmatter = Frontmatter();
    if (m_buffer->frontmatter->aliases == aliases) return;
    m_buffer->frontmatter->aliases = aliases;
    m_dirty = true;
}

Notebook::Saved Notebook::saveNow(QString *error) {
    if (!m_dirty) return Saved::Clean;
    if (m_vault.isNull() || !m_buffer) { m_dirty = false; return Saved::Clean; }
    VaultError problem;
    if (!m_vault.write(*m_buffer, &problem)) {
        m_notSaving = problem.message;
        if (error) *error = problem.message;
        return Saved::Failed;
    }
    m_dirty = false;
    m_onDisk = m_buffer->toText();
    m_index.update(*m_buffer);
    m_lexical = search::Bm25::build(m_index);
    m_notSaving.reset();
    m_diverged.reset();
    m_vanished.reset();
    return Saved::Written;
}

// ---- links and search -----------------------------------------------------

QStringList Notebook::linkCandidates(const QString &query) const {
    QStringList out;
    for (const search::TitleMatch &m : search::byTitle(m_index, query, 16)) {
        if (m_open && m.id == *m_open) continue;
        out.append(m.id.title());
    }
    return out;
}

Resolution Notebook::resolveLink(const QString &target) const {
    return m_index.resolve(target, m_open ? &*m_open : nullptr);
}

QVector<Backlink> Notebook::backlinksOfOpenNote() const {
    return m_open ? m_index.backlinks(*m_open) : QVector<Backlink>();
}

QVector<Hit> Notebook::search(const QString &query, Mode mode, bool *wantsEmbedding) const {
    QVector<Hit> hits;
    if (wantsEmbedding) *wantsEmbedding = false;
    if (mode == Mode::Title) {
        for (const search::TitleMatch &m : search::byTitle(m_index, query, 30)) {
            Hit h;
            h.id = m.id.str();
            h.title = m.id.title();
            h.detail = m.id.folder();
            hits.append(h);
        }
        return hits;
    }
    const bool embedded = m_queryVector && m_queryVector->first == query;
    if (wantsEmbedding) *wantsEmbedding = !embedded;
    const QVector<float> vector = embedded ? m_queryVector->second : QVector<float>();
    for (const search::Hit &hit : search::hybrid(m_index, m_lexical, &m_vectors, vector, query, 30)) {
        Hit h;
        h.id = hit.id.str();
        h.title = hit.id.title();
        h.highlight = search::highlightOf(hit.snippet, query);
        h.detail = hit.snippet;
        h.score = QString::number(double(hit.score * 30.0f), 'f', 2);
        hits.append(h);
    }
    return hits;
}

// ---- attachments ----------------------------------------------------------

QStringList Notebook::attachFiles(const QStringList &paths, QStringList *failed) {
    QStringList names;
    for (const QString &path : paths) {
        if (const auto name = m_vault.addAttachment(path)) names.append(*name);
        else if (failed) failed->append(path);
    }
    return names;
}

std::optional<QString> Notebook::attachBytes(const QString &name, const QByteArray &bytes) {
    if (m_vault.isNull()) return std::nullopt;
    return m_vault.addAttachmentBytes(name, bytes);
}

QStringList Notebook::unusedAttachments() const {
    if (m_vault.isNull()) return {};
    const QSet<QString> used = m_index.referencedAttachments();
    QStringList out;
    for (const QString &name : QDir(m_vault.folderPath(QLatin1String(ATTACHMENTS_DIR))).entryList(QDir::Files, QDir::Name))
        if (!used.contains(name)) out.append(name);
    return out;
}

// ---- notes and folders ----------------------------------------------------

QString Notebook::currentFolder() const {
    if (m_targetFolder) return *m_targetFolder;
    if (m_open) return m_open->folder();
    return QString();
}

NoteId Notebook::uniqueId(const QString &folder, const QString &title) const {
    auto build = [&](const QString &name) { return NoteId::fromRelative(folder.isEmpty() ? name + QStringLiteral(".md") : folder + u'/' + name + QStringLiteral(".md")); };
    NoteId id = build(title);
    int attempt = 2;
    while (QFileInfo::exists(m_vault.pathOf(id))) id = build(title + u' ' + QString::number(attempt++));
    return id;
}

std::optional<NoteId> Notebook::createNoteIn(const QString &folder, const QString &title, QString *error) {
    if (m_vault.isNull()) { if (error) *error = QStringLiteral("no vault"); return std::nullopt; }
    const NoteId id = uniqueId(folder, title);
    VaultError problem;
    const auto note = m_vault.create(id, QString(), &problem);
    if (!note) { if (error) *error = problem.message; return std::nullopt; }
    m_index.update(*note);
    m_lexical = search::Bm25::build(m_index);
    loadNote(id);
    return id;
}

Notebook::Renamed Notebook::renameNote(const QString &title) {
    Renamed r;
    if (m_vault.isNull() || !m_open) return r;
    const NoteId from = *m_open;
    if (from.title() == title) return r;
    const NoteId to = NoteId::fromRelative(from.folder().isEmpty() ? title + QStringLiteral(".md") : from.folder() + u'/' + title + QStringLiteral(".md"));
    QVector<NoteId> inbound;
    for (const Backlink &b : m_index.backlinks(from)) if (!inbound.contains(b.from)) inbound.append(b.from);
    VaultError problem;
    if (!m_vault.rename(from, to, &problem)) { r.kind = Renamed::Failed; r.error = problem.message; return r; }
    m_index.rename(from, to);
    int links = 0;
    for (const NoteId &id : inbound) {
        auto note = m_vault.read(id);
        if (!note) continue;
        const auto body = md::rewriteTarget(note->body, from.title(), to.title());
        if (!body) continue;
        note->body = *body;
        if (m_vault.write(*note)) { m_index.update(*note); ++links; }
    }
    m_lexical = search::Bm25::build(m_index);
    m_open = to;
    if (m_buffer) m_buffer->id = to;
    m_config.lastNote = to.str();
    r.kind = Renamed::Done;
    r.to = to;
    r.links = links;
    return r;
}

std::optional<NoteId> Notebook::deleteOpenNote(QString *error) {
    if (m_vault.isNull() || !m_open) return std::nullopt;
    const NoteId id = *m_open;
    m_dirty = false;
    m_buffer.reset();
    m_onDisk.reset();
    m_open.reset();
    clearAlerts();
    VaultError problem;
    if (!m_vault.remove(id, &problem)) { if (error) *error = problem.message; return std::nullopt; }
    m_index.remove(id);
    m_lexical = search::Bm25::build(m_index);
    m_config.lastNote.reset();
    return id;
}

std::optional<QString> Notebook::createFolder(const QString &parent, const QString &name, QString *error) {
    if (m_vault.isNull()) return std::nullopt;
    const QString path = join(parent, name);
    VaultError problem;
    if (!m_vault.createFolder(path, &problem)) { if (error) *error = problem.message; return std::nullopt; }
    for (const QString &ancestor : tree::ancestors(path)) m_expanded.insert(ancestor);
    m_targetFolder = path;
    return path;
}

bool Notebook::deleteFolder(const QString &path, QString *error) {
    if (m_vault.isNull()) return false;
    VaultError problem;
    if (!m_vault.deleteFolder(path, &problem)) { if (error) *error = problem.message; return false; }
    m_expanded.remove(path);
    if (m_targetFolder && tree::isWithin(path, *m_targetFolder)) m_targetFolder.reset();
    return true;
}

bool Notebook::renameFolder(const QString &path, const QString &name, QString *error) {
    const int slash = path.lastIndexOf(u'/');
    const QString parent = slash < 0 ? QString() : path.left(slash);
    return relocateFolder(path, join(parent, name), error);
}

bool Notebook::relocateFolder(const QString &from, const QString &to, QString *error) {
    if (m_vault.isNull()) return false;
    if (from == to) return true;
    VaultError problem;
    if (!m_vault.moveFolder(from, to, &problem)) { if (error) *error = problem.message; return false; }
    auto moved = [&](const QString &path) -> std::optional<QString> {
        if (path == from) return to;
        if (path.startsWith(from + u'/')) return to + path.mid(from.size());
        return std::nullopt;
    };
    QSet<QString> expanded;
    for (const QString &path : m_expanded) expanded.insert(moved(path).value_or(path));
    for (const QString &ancestor : tree::ancestors(to)) expanded.insert(ancestor);
    m_expanded = expanded;
    if (m_targetFolder) m_targetFolder = moved(*m_targetFolder).value_or(*m_targetFolder);
    std::optional<NoteId> reopen;
    if (m_open) if (const auto m = moved(m_open->str())) reopen = NoteId::fromRelative(*m);
    rescan();
    if (reopen) { m_config.lastNote = reopen->str(); loadNote(*reopen); }
    return true;
}

Notebook::Moved Notebook::moveNote(const NoteId &id, const QString &destination) {
    Moved m;
    if (m_vault.isNull()) return m;
    if (id.folder() == destination) return m;
    const NoteId to = NoteId::fromRelative(join(destination, id.title() + QStringLiteral(".md")));
    const bool wasOpen = m_open && *m_open == id;
    VaultError problem;
    if (!m_vault.rename(id, to, &problem)) { m.kind = Moved::Failed; m.error = problem.message; return m; }
    m_index.rename(id, to);
    m_lexical = search::Bm25::build(m_index);
    if (wasOpen) {
        m_open = to;
        if (m_buffer) m_buffer->id = to;
        m_config.lastNote = to.str();
    }
    expandTo(to);
    m.kind = Moved::Done;
    m.to = to;
    m.destination = destination;
    return m;
}

bool Notebook::appendTo(const NoteId &id, const QString &text, QString *error) {
    if (m_vault.isNull()) return false;
    // The open note is edited in memory so the tick writes it; anything else
    // goes straight through the vault.
    if (m_open && *m_open == id && m_buffer) {
        QString body = m_buffer->body;
        if (!body.isEmpty() && !body.endsWith(u'\n')) body += u'\n';
        body += text + u'\n';
        m_buffer->body = body;
        m_dirty = true;
        return true;
    }
    auto note = m_vault.read(id);
    if (!note) note = Note::fromText(id, QString());
    if (!note->body.isEmpty() && !note->body.endsWith(u'\n')) note->body += u'\n';
    note->body += text + u'\n';
    VaultError problem;
    if (!m_vault.write(*note, &problem)) { if (error) *error = problem.message; return false; }
    m_index.update(*note);
    m_lexical = search::Bm25::build(m_index);
    return true;
}

// ---- sync -----------------------------------------------------------------

std::optional<std::pair<Vault, sync::Snapshot>> Notebook::syncInput() const {
    if (m_vault.isNull()) return std::nullopt;
    return std::make_pair(m_vault, sync::loadBase(sync::defaultBasePath(m_vault.root())));
}

sync::Report Notebook::absorbSync(sync::Incoming incoming, const QString &from, const QString &date) {
    m_lastPass = QDateTime::currentDateTime();
    m_lastSyncError.reset();
    if (m_vault.isNull()) return sync::Report();
    const std::optional<NoteId> protect = m_dirty ? m_open : std::nullopt;
    auto [agreed, report] = sync::apply(m_vault, std::move(incoming), protect ? &*protect : nullptr, from, date);
    sync::saveBase(agreed, sync::defaultBasePath(m_vault.root()));
    if (!report.isQuiet()) {
        m_lastChange = report;
        rescan();
        m_conflicts += report.conflicted;
        absorbExternalChanges();
    }
    return report;
}

void Notebook::recordSyncFailure(const QString &error) {
    m_lastPass = QDateTime::currentDateTime();
    m_lastSyncError = error;
}

SyncStatus Notebook::syncStatus() const {
    SyncStatus s;
    if (const auto server = syncServer()) s.server = server->first;
    if (!m_vault.isNull()) s.vault = m_vault.root();
    s.notesHere = m_index.size();
    s.notesAgreed = m_vault.isNull() ? 0 : sync::loadBase(sync::defaultBasePath(m_vault.root())).size();
    s.lastPass = m_lastPass;
    s.lastChange = m_lastChange;
    s.lastError = m_lastSyncError;
    s.vectors = m_vectors.size();
    s.embeddingServer = embeddingUrl();
    return s;
}

} // namespace brain

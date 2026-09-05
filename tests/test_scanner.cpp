#include <QtTest>

#include "scanner.h"

using namespace brain::md;

namespace {
QStringList styles(const QString &text) {
    QStringList out;
    for (const Span &s : parse(text).spans) {
        QString name;
        switch (s.style) {
        case Style::Heading: name = QStringLiteral("h%1").arg(s.level); break;
        case Style::Bold: name = "bold"; break;
        case Style::Italic: name = "italic"; break;
        case Style::Strikethrough: name = "strike"; break;
        case Style::Code: name = "code"; break;
        case Style::CodeBlock: name = "codeblock"; break;
        case Style::Quote: name = "quote"; break;
        case Style::ListItem: name = QStringLiteral("list%1").arg(s.level); break;
        case Style::Link: name = "link"; break;
        case Style::WikiLink: name = "wikilink"; break;
        case Style::Embed: name = "embed"; break;
        case Style::Tag: name = "tag"; break;
        case Style::Task: name = s.ticked ? "task-done" : "task"; break;
        case Style::Rule: name = "rule"; break;
        case Style::TableRow: name = "row"; break;
        case Style::TableDelimiter: name = "delim"; break;
        case Style::Frontmatter: name = "fm"; break;
        case Style::Comment: name = "comment"; break;
        }
        out.append(name + ":" + text.mid(s.start, s.end - s.start));
    }
    return out;
}
QStringList markers(const QString &text) {
    QStringList out;
    for (const Marker &m : parse(text).markers) out.append(text.mid(m.start, m.end - m.start));
    return out;
}
}

class TestScanner : public QObject {
    Q_OBJECT
private slots:
    void headingsMarkTheirHashes() {
        QCOMPARE(styles("## Title"), QStringList{"h2:Title"});
        QCOMPARE(markers("## Title"), QStringList{"## "});
        QVERIFY(styles("####### seven").isEmpty());
        QVERIFY(styles("#nospace").size() == 1);   // a tag, not a heading
    }
    void emphasisHugsItsContent() {
        QCOMPARE(styles("a **b** c"), QStringList{"bold:b"});
        QCOMPARE(markers("a **b** c"), (QStringList{"**", "**"}));
        QVERIFY(styles("a * b * c").isEmpty());
        QCOMPARE(styles("~~x~~ _y_"), (QStringList{"strike:x", "italic:y"}));
        QCOMPARE(styles("***both***"), (QStringList{"bold:both", "italic:both"}));
    }
    void codeWinsOverEverything() {
        QCOMPARE(styles("`[[not a link]]`"), QStringList{"code:[[not a link]]"});
    }
    void wikilinksAndEmbeds() {
        QCOMPARE(styles("see [[Target|shown]]"), QStringList{"wikilink:shown"});
        QCOMPARE(markers("see [[Target|shown]]"), (QStringList{"[[Target|", "]]"}));
        QCOMPARE(styles("![[pic.png]]"), QStringList{"embed:pic.png"});
        QVERIFY(styles("[[]]").isEmpty());
    }
    void linksAndUrls() {
        QCOMPARE(styles("[label](https://x.y)"), QStringList{"link:label"});
        QCOMPARE(styles("go to https://example.com/a."), QStringList{"link:https://example.com/a"});
        QVERIFY(styles("nothttps://x").isEmpty());
    }
    void tags() {
        QCOMPARE(styles("about #project/brain and #404 or C#"), QStringList{"tag:#project/brain"});
        QCOMPARE(styles("#tag/ trailing"), QStringList{"tag:#tag"});
    }
    void listsNestByIndentAndTasksStayVisible() {
        const QString text = "- a\n  - [x] b\n1. c";
        QCOMPARE(styles(text), (QStringList{"list0:a", "task-done:[x]", "list1: b", "list0:c"}));   // the space belongs to the content
        QCOMPARE(markers(text), QStringList{"  "});   // only the indent is syntax
    }
    void quotesRulesAndFences() {
        QCOMPARE(styles("> q"), QStringList{"quote:q"});
        QCOMPARE(styles("Prose.\n---"), QStringList{"rule:---"});
        const Parsed p = parse("```rs\nlet x;\n```\nafter");
        QCOMPARE(p.lineStates, (QVector<LineState>{LineState::Normal, LineState::Fence, LineState::Fence, LineState::Normal}));
        QCOMPARE(styles("```\ncode\n```"), QStringList{"codeblock:code"});
    }
    void tablesNeedADelimiterRow() {
        QCOMPARE(styles("| a | b |\n|---|---|\n| 1 | 2 |"), (QStringList{"row:| a | b |", "delim:|---|---|", "row:| 1 | 2 |"}));
        QVERIFY(styles("| yes | no |").isEmpty());
    }
    void frontmatterOnlyAtTheTop() {
        QCOMPARE(styles("---\ntags: [a]\n---\nbody"), (QStringList{"fm:---", "fm:tags: [a]", "fm:---"}));
        QCOMPARE(styles("\n---\nx"), QStringList{"rule:---"});
    }
    void inlineCommentsAreRecessedNotProse() {
        QCOMPARE(styles("text #tag <!-- fact --> after"), (QStringList{"tag:#tag", "comment:<!-- fact -->"}));
    }
    void htmlCommentsAreTheirOwnLine() {
        QCOMPARE(styles("<!-- familiar fact 2026-08-06 -->"), QStringList{"comment:<!-- familiar fact 2026-08-06 -->"});
        QCOMPARE(strip("a\n<!-- x -->\nb"), QStringLiteral("a\n\nb"));
    }
    void stripRemovesSyntaxAndBullets() {
        QCOMPARE(strip("# Shopping\n- **milk**\n| a | b |\n|---|---|\n"), QStringLiteral("Shopping\nmilk\n a b \n\n"));
        QCOMPARE(strip("[[Target|shown]] #tag"), QStringLiteral("shown #tag"));
    }
    void extractFindsLinksEmbedsAndTags() {
        const Extracted e = extract("[[A]] and [[B|shown]] ![[p.png|300]] #t/u `[[no]]`");
        QCOMPARE(e.links.size(), 2);
        QCOMPARE(e.links[0].target, QStringLiteral("A"));
        QCOMPARE(e.links[1].target, QStringLiteral("B"));
        QCOMPARE(*e.links[1].display, QStringLiteral("shown"));
        QCOMPARE(e.links[0].start, 0);
        QCOMPARE(e.links[0].end, 5);
        QCOMPARE(e.embeds.size(), 1);
        QCOMPARE(e.embeds[0].target, QStringLiteral("p.png"));
        QCOMPARE(e.tags.size(), 1);
        QCOMPARE(e.tags[0].name, QStringLiteral("t/u"));
    }
    void rewriteTargetRepointsLinks() {
        QCOMPARE(*rewriteTarget("See [[Old]] today.", "Old", "New"), QStringLiteral("See [[New]] today."));
        QCOMPARE(*rewriteTarget("See [[Old|the old thing]].", "Old", "New"), QStringLiteral("See [[New|the old thing]]."));
        QCOMPARE(*rewriteTarget("See [[Meetings/Old]].", "Old", "New"), QStringLiteral("See [[New]]."));
        QCOMPARE(*rewriteTarget("![[old.png|300]]", "old.png", "new.png"), QStringLiteral("![[new.png|300]]"));
        QVERIFY(!rewriteTarget("`[[Old]]` in code", "Old", "New"));
        QCOMPARE(*rewriteTarget("🎉 See [[Café]] 🎉", "Café", "Tearoom"), QStringLiteral("🎉 See [[Tearoom]] 🎉"));
    }
    void listEnterContinuesOrEnds() {
        QCOMPARE(listEnter("- item")->prefix, QStringLiteral("- "));
        QCOMPARE(listEnter("  3) item")->prefix, QStringLiteral("  4) "));
        QCOMPARE(listEnter("- [x] done")->prefix, QStringLiteral("- [ ] "));
        QVERIFY(listEnter("- ")->endList);
        QVERIFY(listEnter("- [ ] ")->endList);
        QVERIFY(!listEnter("prose"));
    }
    void renumberFixesGaps() {
        const auto edits = renumber("1. a\n3. b\n\n7. c");
        QCOMPARE(edits.size(), 2);
        QCOMPARE(edits[0].number, 2u);
        QCOMPARE(edits[1].number, 3u);
        QVERIFY(renumber("3. a\n4. b").isEmpty());
        QVERIFY(renumber("```\n1. a\n3. b\n```").isEmpty());
    }
    void everyFormatWritesSyntaxTheScannerStyles() {
        for (Format f : allFormats()) {
            const Edit e = editFor(f);
            QString written;
            if (e.kind == Edit::Wrap) written = e.before + "sample" + e.after;
            else if (e.kind == Edit::Prefix) written = e.prefix + "sample";
            else written = "Prose.\n" + e.text;
            QVERIFY2(!parse(written).spans.isEmpty() || f == Format::CodeBlock, qPrintable(labelOf(f)));
            QVERIFY(!labelOf(f).isEmpty());
            QVERIFY(!syntaxOf(f).isEmpty());
        }
        QCOMPARE(allFormats().size(), 15);
    }
};

QTEST_APPLESS_MAIN(TestScanner)
#include "test_scanner.moc"

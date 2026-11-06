// Builds a vault full of edge cases for the parity test. Usage: node make-fixture.js <dir>
const fs = require('fs');
const path = require('path');
const root = process.argv[2];
if (!root) { console.error('usage: make-fixture.js <dir>'); process.exit(2); }
fs.rmSync(root, { recursive: true, force: true });
const w = (rel, content) => {
  const p = path.join(root, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
};

// Duplicate basenames: ids become folder paths ("./Note" at the root).
w('Note.md', '#root-note\n[[Other]]');
w('a/Note.md', '---\ntags: [alpha]\n---\n[[Note]]');
w('b/deep/Note.md', 'links [[a/Note]] and [[B/DEEP/NOTE]]');

// Link resolution: exact, case-insensitive, by path, by last segment, alias, heading,
// self-links, repeated links, ghosts with different case.
w('Other.md', '[[other]] [[Other|alias]] [[Other#Heading]] [[x/y/Hub]] [[Missing]] [[missing]] [[Missing]]');
w('hubs/Hub.md', '---\ntags:\n  - hub\n  - "quoted"\n  - \'single\'\n---\n[[Hub]] [[Other]] [[ Spaced ]]');
w('Spaced.md', 'target of a link with spaces');
w('Self.md', '[[Self]] [[self]]');

// Embeds and attachments.
w('media/Embeds.md', '![[pic.png]] ![[Diagram.excalidraw]] [[file.pdf]] ![[song.MP3]] ![[Other.md]] ![[Spaced]] ![[notes.txt]] [[Other.md]] [[data.csv]]');

// Tags: flow and block lists, inline, nested, duplicates, things that aren't tags.
w('tags/Flow.md', '---\ntitle: x\ntags: [one, "two", \'three\', one]\n---\n#four #one #nested/deep-tag #5no x#no');
w('tags/Block.md', '---\ntags:\n- first\n  - second\n---\n#third');
w('tags/CRLF.md', '---\r\ntags:\r\n  - crlf\r\n---\r\nbody #crlf-inline\r\n[[Other]]');
w('tags/Unclosed.md', '---\ntags: [nope]\nno closing fence\n#inline-only');
w('tags/BOM.md', '﻿---\ntags: [bom]\n---\n#after-bom');
w('tags/Empty.md', '');
w('tags/Hash at start.md', '#start [[Note]]');
w('tags/Unicode space.md', 'text #nbsp-tag and　#ideographic and\u0085#nel-not-js-space');

// Unicode names and case folding.
w('Café.md', '[[café]] [[CAFÉ]]');
w('日本語.md', '#日本 #jp [[Café]]');
w('🚀 Launch.md', '[[日本語]] [[ΣΟΦΙΑ]]');
w('Σοφια.md', 'greek');
w('｛wide｝.md', 'sorts after the rocket in UTF-16 order');

// Folders whose names look like files, and multi-line link text.
w('notes.v2/Inside.md', '[[Other\nwith newline]] [[Spaced]]');

// Hidden and ignored paths (run with --ignore templates --ignore archive/old).
w('.obsidian/Hidden.md', '[[Other]]');
w('.trash/Gone.md', '[[Other]]');
w('sub/.secret.md', '[[Other]]');
w('templates/Template.md', '[[Other]]');
w('deep/templates/Nested template.md', '[[Other]]');
w('archive/old/Old.md', '[[Other]]');
w('archive/Kept.md', '[[Old]] [[Template]]');

// Not markdown.
w('Upper.MD', '[[Other]]');
w('plain.txt', '[[Other]]');
w('image.png', 'not really');

// Symlinks are never followed.
fs.symlinkSync(path.join(root, 'Other.md'), path.join(root, 'Link to other.md'));
fs.symlinkSync(path.join(root, 'hubs'), path.join(root, 'hubs-link'));

// A long note for the excerpt.
w('Long.md', '---\na: 1\n---\n# Heading\n' + 'Some *emphasis* and `code` with [[Other|an alias]] and [a link](http://x). '.repeat(8) + '\n```js\nhidden code\n```\nend');

console.log(`fixture written to ${root}`);

import { readFileSync, writeFileSync } from 'fs';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const IN   = join(ROOT, 'docs', 'arch.excalidraw');
const OUT  = join(ROOT, 'docs', 'arch.md');

const data = JSON.parse(readFileSync(IN, 'utf8'));
const els  = data.elements.filter(function(e) { return !e.isDeleted; });

const rects  = els.filter(function(e) { return e.type === 'rectangle' || e.type === 'frame'; });
const texts  = els.filter(function(e) { return e.type === 'text'; });
const arrows = els.filter(function(e) { return e.type === 'arrow'; });

function contains(rect, text) {
  const PAD = 80;
  return (
    text.x >= rect.x - PAD &&
    text.y >= rect.y - PAD &&
    text.x + text.width  <= rect.x + rect.width  + PAD &&
    text.y + text.height <= rect.y + rect.height + PAD
  );
}

function makeId(str) {
  return str
    .replace(/^#+\s*/, "")
    .replace(/[^a-zA-Z0-9]/g, "_")
    .replace(/_+/g, "_")
    .replace(/^_|_$/g, "");
}

function escapeMermaid(str) {
  return str.replace(/"/g, "'");
}

var nodes = [];
var assignedTextIds = new Set();

if (rects.length > 0) {
  rects.forEach(function(rect) {
    var inside  = texts.filter(function(t) { return contains(rect, t); });
    var heading = inside.find(function(t) { return /^#+\s/.test(t.text); });
    var body    = inside.filter(function(t) { return !(/^#+\s/.test(t.text)); });
    inside.forEach(function(t) { assignedTextIds.add(t.id); });

    var rawLabel = heading ? heading.text.replace(/^#+\s*/, "") : ("node_" + rect.id.slice(0, 6));
    var id       = makeId(rawLabel);
    var desc     = body.map(function(t) { return t.text; }).join('\n');
    var label    = desc ? (rawLabel + '\n\n' + desc) : rawLabel;
    nodes.push({ id: id, label: label, rectId: rect.id });
  });

  texts.forEach(function(t) {
    if (!assignedTextIds.has(t.id)) {
      var id = makeId(t.text.split('\n')[0]);
      if (id) nodes.push({ id: id, label: t.text, rectId: null });
    }
  });
} else {
  var headings  = texts.filter(function(t) { return /^#+\s/.test(t.text); });
  var bodyTexts = texts.filter(function(t) { return !(/^#+\s/.test(t.text)); });
  headings.forEach(function(h) {
    var rawLabel = h.text.replace(/^#+\s*/, "");
    var id  = makeId(rawLabel);
    var desc = bodyTexts
      .filter(function(b) { return Math.abs(b.x - h.x) < 300; })
      .map(function(b) { return b.text; })
      .join('\n');
    var label = desc ? (rawLabel + '\n\n' + desc) : rawLabel;
    nodes.push({ id: id, label: label, rectId: null });
  });
}

nodes.sort(function(a, b) {
  var ra = rects.find(function(r) { return r.id === a.rectId; });
  var rb = rects.find(function(r) { return r.id === b.rectId; });
  return ((ra && ra.x) || 0) - ((rb && rb.x) || 0);
});

var edges = [];
arrows.forEach(function(arrow) {
  var startId = arrow.startBinding && arrow.startBinding.elementId;
  var endId   = arrow.endBinding   && arrow.endBinding.elementId;
  if (!startId || !endId) return;
  var from = nodes.find(function(n) { return n.rectId === startId; });
  var to   = nodes.find(function(n) { return n.rectId === endId;   });
  if (!from || !to) return;
  var edgeLabel = "";
  if (arrow.boundElements) {
    var labelEl = els.find(function(e) {
      return e.type === 'text' && arrow.boundElements.some(function(b) { return b.id === e.id; });
    });
    if (labelEl) edgeLabel = labelEl.text;
  }
  edges.push({ from: from.id, to: to.id, label: edgeLabel });
});

var lines = ['```mermaid', 'flowchart LR'];
nodes.forEach(function(n) {
  lines.push('  ' + n.id + '["' + escapeMermaid(n.label) + '"]');
});
edges.forEach(function(e) {
  if (e.label) {
    lines.push('  ' + e.from + ' -->|"' + escapeMermaid(e.label) + '"| ' + e.to);
  } else {
    lines.push('  ' + e.from + ' --> ' + e.to);
  }
});
lines.push('```');

var output = '# Architecture Diagram\n\n' + lines.join('\n') + '\n';
writeFileSync(OUT, output);
console.log('Written: ' + OUT);

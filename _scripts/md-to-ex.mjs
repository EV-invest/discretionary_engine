import { readFileSync, writeFileSync } from 'fs';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';
import { JSDOM } from 'jsdom';

// Set up DOM globals before loading mermaid (needs DOMPurify + SVG layout)
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.window   = dom.window;
global.document = dom.window.document;
global.DOMParser = dom.window.DOMParser;
Object.defineProperty(global, 'navigator', { value: dom.window.navigator, writable: true, configurable: true });
if (dom.window.SVGElement) {
  dom.window.SVGElement.prototype.getBBox = function() {
    return { x: 0, y: 0, width: 100, height: 50 };
  };
}

// Dynamic import AFTER globals are set
const { parseMermaidToExcalidraw } = await import('@excalidraw/mermaid-to-excalidraw');

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const IN   = join(ROOT, 'docs', 'arch.md');
const OUT  = join(ROOT, 'docs', 'arch.excalidraw');

const src = readFileSync(IN, 'utf8');
const match = src.match(/```mermaid\n([\s\S]*?)```/);
if (!match) {
  console.error('No mermaid code block found in ' + IN);
  process.exit(1);
}

const mermaidSrc = match[1].trim();
console.log('Parsing mermaid...');

const result = await parseMermaidToExcalidraw(mermaidSrc);

const excalidraw = {
  type: 'excalidraw',
  version: 2,
  source: 'mermaid-to-excalidraw',
  elements: result.elements || [],
  appState: {
    gridSize: null,
    gridStep: 5,
    viewBackgroundColor: '#ffffff'
  },
  files: result.files || {}
};

writeFileSync(OUT, JSON.stringify(excalidraw, null, 2));
console.log('Written: ' + OUT);

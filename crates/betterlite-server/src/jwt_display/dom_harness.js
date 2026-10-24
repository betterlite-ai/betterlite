const observed = { clipboard: [], alerts: [], errors: [] };
const listeners = new Map();
let clipboardFails = false;

class Element {
  constructor(tag, text) {
    Object.assign(this, { tag, className: '', textContent: text, children: [] });
  }
  set innerHTML(markup) {
    if (markup !== '') {
      throw new Error(`markup assigned: ${markup}`);
    }
    this.children = [];
  }
  appendChild(child) {
    this.children.push(child);
  }
}

const elements = new Map([
  ['parsedByProxy', new Element('div', '')],
  ['rawClaims', new Element('pre', 'Loading...')],
  ['accessTokenClaims', new Element('pre', 'Loading...')],
]);

globalThis.document = {
  getElementById: (id) => elements.get(id),
  createElement: (tag) => new Element(tag, ''),
  addEventListener: (type, listener) => listeners.set(type, listener),
};
globalThis.navigator = { clipboard: { writeText: (text) => {
  observed.clipboard.push(text);
  return clipboardFails ? Promise.reject(new Error('denied')) : Promise.resolve();
} } };
globalThis.alert = (message) => observed.alerts.push(message);
globalThis.console = { error: (...parts) => observed.errors.push(parts.map(String).join('|')) };

const fire = (type) => listeners.get(type)();
const outline = (node) => {
  const inner = node.children.map(outline).join(' ');
  return `${node.tag}.${node.className}${JSON.stringify(node.textContent)}${inner === '' ? '' : `[${inner}]`}`;
};
const observe = () => JSON.stringify({
  parsed: elements.get('parsedByProxy').children.map(outline),
  raw: elements.get('rawClaims').textContent,
  access: elements.get('accessTokenClaims').textContent,
  ...observed,
});

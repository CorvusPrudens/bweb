// bweb's JS-side registry of managed DOM nodes, keyed by the Rust-side
// `NodeId`. Rust holds lazy handles and fetches through `get_node` on first
// deref; `entities` is the reverse node → packed-Entity-bits map used for
// event hit-testing. Nodes are adopted when Rust creates them and must be
// released through `remove_node` on despawn — the registry holds strong
// references, so a missed release is a leak (see `live_nodes`).

const nodes = new Map();
const entities = new WeakMap();

export function adopt(id, node, entityBits) {
  nodes.set(id, node);
  if (entityBits !== undefined) {
    entities.set(node, entityBits);
  }
}

export function get_node(id) {
  return nodes.get(id);
}

export function remove_node(id) {
  const node = nodes.get(id);
  if (node === undefined) return;
  // `remove` exists on Element and CharacterData but not on Document.
  node.remove?.();
  nodes.delete(id);
}

export function lookup_entity(node) {
  return entities.get(node);
}

export function nearest_entity(node) {
  let current = node;
  while (current !== null) {
    const entity = entities.get(current);
    if (entity !== undefined) return entity;
    current = current.parentNode;
  }
  return undefined;
}

export function live_nodes() {
  return nodes.size;
}

// Reverse-lookup registration for a node created by the interpreter. Entity
// bits arrive as two u32 words (f64 can't ride a Uint32Array); the all-ones
// pair means "skip" (generation overflow guard on the Rust side).
function register(node, lo, hi) {
  if (lo === 0xffffffff && hi === 0xffffffff) return;
  entities.set(node, hi * 0x100000000 + lo);
}

// Execute one flush of the Rust-side DomCommandBuffer. Opcode layouts must
// match `mod op` in src/dom/registry.rs exactly. String operands are
// (offset, len) pairs into `strings`, in UTF-16 code units. Ops whose target
// id has no registry entry are skipped: that covers NOP-patched creates
// (spawn+despawn in one tick) without any bookkeeping here.
export function interpret(ops, strings) {
  const str = (i) => strings.substring(ops[i], ops[i] + ops[i + 1]);
  const len = ops.length;
  let i = 0;
  try {
    while (i < len) {
      switch (ops[i]) {
        case 0: { // Nop(skip)
          i += 2 + ops[i + 1];
          break;
        }
        case 1: { // CreateElement(id, tag, entLo, entHi)
          const node = document.createElement(str(i + 2));
          nodes.set(ops[i + 1], node);
          register(node, ops[i + 4], ops[i + 5]);
          i += 6;
          break;
        }
        case 2: { // CreateElementNs(id, ns, tag, entLo, entHi)
          const node = document.createElementNS(str(i + 2), str(i + 4));
          nodes.set(ops[i + 1], node);
          register(node, ops[i + 6], ops[i + 7]);
          i += 8;
          break;
        }
        case 3: { // CreateText(id, text, entLo, entHi)
          const node = document.createTextNode(str(i + 2));
          nodes.set(ops[i + 1], node);
          register(node, ops[i + 4], ops[i + 5]);
          i += 6;
          break;
        }
        case 4: { // SetText(id, text)
          const node = nodes.get(ops[i + 1]);
          if (node !== undefined) node.data = str(i + 2);
          i += 4;
          break;
        }
        case 5: { // SetAttribute(id, name, value)
          nodes.get(ops[i + 1])?.setAttribute(str(i + 2), str(i + 4));
          i += 6;
          break;
        }
        case 6: { // RemoveAttribute(id, name)
          nodes.get(ops[i + 1])?.removeAttribute(str(i + 2));
          i += 4;
          break;
        }
        case 7: { // AddClass(id, class)
          nodes.get(ops[i + 1])?.classList.add(str(i + 2));
          i += 4;
          break;
        }
        case 8: { // SetPropertyStr(id, name, value)
          const node = nodes.get(ops[i + 1]);
          if (node !== undefined) node[str(i + 2)] = str(i + 4);
          i += 6;
          break;
        }
        case 9: { // SetPropertyBool(id, name, value)
          const node = nodes.get(ops[i + 1]);
          if (node !== undefined) node[str(i + 2)] = ops[i + 4] !== 0;
          i += 5;
          break;
        }
        case 10: { // SetInnerHtml(id, html)
          const node = nodes.get(ops[i + 1]);
          if (node !== undefined) node.innerHTML = str(i + 2);
          i += 4;
          break;
        }
        case 11: { // Append(parent, child)
          const parent = nodes.get(ops[i + 1]);
          const child = nodes.get(ops[i + 2]);
          if (parent !== undefined && child !== undefined) parent.appendChild(child);
          i += 3;
          break;
        }
        case 12: { // InsertBefore(parent, child, anchor)
          const parent = nodes.get(ops[i + 1]);
          const child = nodes.get(ops[i + 2]);
          const anchor = nodes.get(ops[i + 3]);
          if (parent !== undefined && child !== undefined && anchor !== undefined) {
            parent.insertBefore(child, anchor);
          }
          i += 4;
          break;
        }
        default:
          throw new Error(`unknown opcode ${ops[i]}`);
      }
    }
  } catch (e) {
    throw new Error(`bweb interpret failed at op index ${i}: ${e}`);
  }
}

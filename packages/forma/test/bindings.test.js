import {test} from 'node:test';import assert from 'node:assert/strict';
import {parse} from '../src/language.js';import {expandStructure} from '../src/component-semantics.js';
test('computed collections retain host objects for keyed two-way edits',()=>{
 const row={id:'a',name:'A'},state={items:[row]};
 const doc=parse('component A { for item in (state.items ?? []) key item.id { TextField { value <-> item.name; } } }');
 const nodes=expandStructure(doc.nodes,{},state);
 assert.equal(nodes[0].environment.locals.item,row);
 nodes[0].environment.locals.item.name='Edited';assert.equal(state.items[0].name,'Edited');
});

import {test} from 'node:test';import assert from 'node:assert/strict';
import {parse} from '../src/language.js';import {expandStructure} from '../src/component-semantics.js';
test('computed collections retain host objects for keyed two-way edits',()=>{
 const row={id:'a',name:'A'},state={items:[row]};
 const doc=parse('component A { for item in (state.items ?? []) key item.id { TextField { value <-> item.name; } } }');
 const nodes=expandStructure(doc.nodes,{},state);
 assert.equal(nodes[0].environment.locals.item,row);
 nodes[0].environment.locals.item.name='Edited';assert.equal(state.items[0].name,'Edited');
});

test('trackOrigins computes match, forwarded, bound, and expression origins in one pass',()=>{
 const doc=parse(`component Card {
  prop title: String = 'Hello';
  prop active: Bool = true;
  TextField {
   forward props { title };
   value <-> state.draft;
   placeholder: props.title.trim();
   match props.active {
    true => { width: 120px; }
    false => { width: 80px; }
   }
  }
 }`);
 const field=doc.nodes[0];
 field.source={file:'Card.forma',line:5};
 field.matches[0].branches[0].propertySources={width:{file:'Card.forma',line:10}};
 const nodes=expandStructure(
  doc.nodes,
  {title:'Hi',active:true},
  {draft:'x'},
  {},
  {trackOrigins:true,propertySources:{title:{file:'Card.forma',line:2}}}
 );
 const origins=nodes[0].propertyOrigins;
 assert.ok(Array.isArray(origins.title)&&origins.title.length>=1);
 assert.ok(origins.value.some(o=>o.label==='state.draft'));
 assert.ok(origins.placeholder.some(o=>o.source?.file==='Card.forma'));
 assert.ok(origins.width.some(o=>o.label.includes('TextField')&&o.source?.line===10));
});

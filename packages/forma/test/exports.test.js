import {test} from 'node:test';
import assert from 'node:assert/strict';
import {parse} from '@themoretheless/tokenizer-forma';
import {parser} from '@themoretheless/tokenizer-forma/forma-parser';
test('public package exports supply matching component and editor frontends',()=>{
 const source="component Hello { Text { text: 'Hello'; } }";
 assert.equal(parse(source).name,'Hello');
 const tree=parser.parse(source);let errors=0;
 tree.iterate({enter(node){if(node.type.isError)errors++;}});
 assert.equal(errors,0);
});

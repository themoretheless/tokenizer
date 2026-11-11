import {test} from 'node:test';
import assert from 'node:assert/strict';
import {parse,lintDocument,formatSource} from '@themoretheless/tokenizer-forma';
import {parser} from '@themoretheless/tokenizer-forma/forma-parser';
test('public package exports supply matching component and editor frontends',()=>{
 const source=`component Hello {
  prop count: Int = 1;
  Text {
   color: #f08a;
   text: '7'.padStart(3, '0');
   width: match props.count {
    -10..=0 => 10vmin;
    < 100 => 20vmax;
    _ => 30px;
   };
  }
 }`;
 assert.equal(parse(source).name,'Hello');
 assert.deepEqual(lintDocument(source),[]);
 const formatted=formatSource(source);
 assert.equal(formatted,formatSource(formatted));
 const tree=parser.parse(source);let errors=0;
 tree.iterate({enter(node){if(node.type.isError)errors++;}});
 assert.equal(errors,0);
});

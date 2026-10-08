import {test} from 'node:test';
import assert from 'node:assert/strict';
import {resolve} from '../src/language.js';
test('design resolution is supplied by the host and remains isolated',()=>{
 const value={expr:'design.title'};
 assert.equal(resolve(value,{},()=> 'first'),'first');
 assert.equal(resolve(value,{},()=> 'second'),'second');
 assert.equal(resolve([value,'!'],{},()=> 'first'),'first !');
 assert.throws(()=>resolve(value,{}),/No design-data resolver/);
});

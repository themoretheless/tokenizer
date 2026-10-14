import {test} from 'node:test';
import assert from 'node:assert/strict';
import {parseExpression,evaluateExpression,serializeValue,expressionReferences} from '../src/expressions.js';
function evaluate(source,state={}){return evaluateExpression(parseExpression(source),(path,{optional})=>{if(path.startsWith('state.')){let out=state;for(const key of path.slice(6).split('.')){if(out==null||!Object.hasOwn(Object(out),key)){if(optional)return null;throw Error(`Missing ${path}`);}out=out[key];}return out;}return {expr:path};});}
test('pure expression precedence, short circuit, numeric diagnostics and formatting',()=>{
  assert.equal(evaluate('2+3*4'),14);assert.equal(evaluate('(2 + 3) * 4'),20);assert.equal(evaluate('17 % 5'),2);assert.equal(evaluate('17%5'),2);
  assert.equal(evaluate('false && state.missing'),false);assert.equal(evaluate('true || state.missing'),true);assert.equal(evaluate('true ? 12 : state.missing'),12);
  assert.equal(evaluate('state.optional ?? 42'),42);assert.equal(evaluate('state.zero ?? 42',{zero:0}),0);
  assert.equal(evaluate("'Count: ' + String(len(state.items))",{items:[1,2,3]}),'Count: 3');assert.equal(evaluate('clamp(20, 0, 10)'),10);assert.equal(evaluate('Number(\'2.5\') + 1'),3.5);
  assert.throws(()=>evaluate('1 / 0'),/конечное/);assert.throws(()=>evaluate("1 + '2'"),/число/);assert.throws(()=>evaluate('1 && true'),/Bool/);assert.throws(()=>evaluate('min()'),/аргументов/);
  assert.throws(()=>parseExpression('fetch(1)'),/чистая функция/);
});
test('templates support nesting, escaping and optional navigation in either quote style',()=>{
  assert.equal(evaluate("'Hello ${state.user?.name ?? 'guest'}'",{}),'Hello guest');
  assert.equal(evaluate('"Hello ${state.user?.name ?? "guest"}"',{}),'Hello guest');
  assert.equal(evaluate("'Literal \\${state.count}; ${state.count}'",{count:7}),'Literal ${state.count}; 7');
  assert.equal(evaluate("'${true ? 'nested ${state.count}' : 'none'}'",{count:7}),'nested 7');
  assert.equal(evaluate('state.items?.[0]?.name ?? \'None\'',{items:[{name:'First'}]}),'First');
  assert.equal(evaluate('state.items?.[0]?.name ?? \'None\'',{}),'None');
  assert.throws(()=>evaluate('state.items[0].constructor',{items:[{}]}),/Недоступное/);
});
test('serialization preserves values, interpolation, optional indexes and expression dependencies',()=>{
  for(const source of ["'Literal \\${value}'","'Count: ${state.count + 1}'",'state.items[0].name',"state.items?.[state.index]?.['odd-key'] ?? 'missing'",'state.count > 1 ? 4 : 5']){
    const parsed=parseExpression(source);assert.deepEqual(parseExpression(serializeValue(parsed)),parsed,source);
  }
  assert.deepEqual(expressionReferences(parseExpression("'${state.items?.[state.index]?.name ?? props.fallback}'")),['state.items','state.index','props.fallback']);
});
test('conversions reject records and ambiguous empty values; indexed member chains are readable',()=>{
  assert.equal(evaluate('state.items[0].address.city',{items:[{address:{city:'Paris'}}]}),'Paris');
  assert.equal(evaluate("state.user?.address.city ?? 'Empty'",{}),'Empty');
  for(const source of ['Number(null)','Number(true)',"Number('')",'String(state.item)',"'Item: ${state.item}'"])
    assert.throws(()=>evaluate(source,{item:{id:1}}));
});
test('equality compares pure data structurally with strict scalar types across runtimes',()=>{
  assert.equal(evaluate('[1, {name: \'A\';}] == [1, {name: \'A\';}]'),true);
  assert.equal(evaluate("{name: 'A'; count: 1;} === {count: 1; name: 'A';}"),true);
  assert.equal(evaluate('[1, 2] != [1, 3]'),true);assert.equal(evaluate("[1] == ['1']"),false);
  assert.equal(evaluate('#ffffff == #ffffff'),true);assert.equal(evaluate("#ffffff == '#ffffff'"),false);
  assert.equal(evaluate("'😀' > '\\uE000'".replace('\\uE000','\uE000')),true);
  assert.equal(evaluate("'Size: ${12px}'"),'Size: 12px');
});
test('optional indexed access skips its index expression when the receiver is absent',()=>{
  assert.equal(evaluate("state.items?.[state.missing] ?? 'None'",{}),'None');
});
test('reference collection walks nested values, keeps first-seen order and stays read-only for shared values',()=>{
  const value={background:{expr:'props.theme'},shadow:[{expr:'state.depth'},{expr:'props.theme'},{expr:'!state.dark'}],radius:4};
  assert.deepEqual(expressionReferences(value),['props.theme','state.depth','state.dark']);
  assert.equal(expressionReferences(value),expressionReferences(value),'a repeated value object reuses one frozen result');
  assert.equal(Object.isFrozen(expressionReferences(value)),true);
  assert.deepEqual(expressionReferences(42),[]);
  assert.deepEqual(expressionReferences('text'),[]);
  assert.deepEqual(expressionReferences({expr:'state.count + 1'}),[]);
});

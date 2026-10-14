import {test} from 'node:test';
import assert from 'node:assert/strict';
import {parse,resolve,validateDesign,designStatePatch} from '../src/language.js';
test('component exposes defaults separately from primitives',()=>{const p=parse("component Button { width: 100; text: 'Кнопка'; fontSize: 16; Rectangle { radius: props.radius; } }");assert.deepEqual(p.defaults,{width:100,text:'Кнопка',fontSize:16});assert.equal(p.nodes.length,1);assert.equal(p.nodes[0].type,'Rectangle');});
test('design attribute, bindings and actions',()=>{const p=parse("#[design('./demo.design.ui')] component Demo { TextInput { value <-> state.query; } Button { clicked -> actions.search(); disabled: !state.loading; } }");assert.deepEqual(p.designs,['./demo.design.ui']);assert.equal(p.nodes[0].bindings.value,'state.query');assert.equal(p.nodes[1].events.clicked,'actions.search');assert.equal(resolve(p.nodes[1].props.disabled,{loading:true}),false);});
test('legacy preview is rejected',()=>assert.throws(()=>parse("preview 'Data' { state: { loading: false; }; }"),/больше не поддерживается/));
test('broken input fails instead of silently dropping source',()=>{assert.throws(()=>parse("component Demo { Text { text: 'Hi'; }"));assert.throws(()=>parse('component Demo {} ???'));assert.throws(()=>parse("component Demo { Text { text: 'a'; text: 'b'; } }"));});
// A design file carries the always-on base overrides plus named states; the panel the designer
// shows is one row per state, so the parse must keep them separate and locatable in the source.
const statesSource=`design Demo {
    Text { key: 'status'; text: 'Готово'; }
    state 'loading' {
        Text { key: 'status'; text: 'Ищем…'; }
        Button { key: 'go'; disabled: true; }
    }
    state 'error' { Text { key: 'status'; text: 'Упало'; } }
}`;
test('design groups overrides into named states',()=>{const d=parse(statesSource);assert.deepEqual(d.overrides,{status:{text:'Готово'}});assert.deepEqual(d.states.map(s=>s.name),['loading','error']);assert.deepEqual(d.states[0].overrides,{status:{text:'Ищем…'},go:{disabled:true}});assert.deepEqual(d.states[0].overrideTypes,{status:'Text',go:'Button'});assert.deepEqual(d.states[1].overrides,{status:{text:'Упало'}});});
test('a state records where it lives in the source',()=>{const [loading]=parse(statesSource).states;assert.equal(statesSource.slice(loading.nameStart,loading.nameEnd),"'loading'");assert.equal(statesSource.slice(loading.start,loading.start+5),'state');assert.equal(statesSource.slice(loading.end-1,loading.end),'}');assert.equal(statesSource.slice(loading.start,loading.end).split('\n')[0],'state \'loading\' {');});
test('a state patches the base property by property',()=>{const d=parse(statesSource);assert.deepEqual(designStatePatch(d),{status:{text:'Готово'}});assert.deepEqual(designStatePatch(d,'loading'),{status:{text:'Ищем…'},go:{disabled:true}});assert.deepEqual(designStatePatch(d,'error'),{status:{text:'Упало'}});assert.throws(()=>designStatePatch(d,'nope'),/Состояние nope не объявлено/);});
test('empty and duplicate state names are rejected',()=>{assert.throws(()=>parse("design Demo { state 'a' {} state 'a' {} }"),/Повторное состояние a/);assert.throws(()=>parse('design Demo { state loading {} }'),/Имя состояния ожидает строку в кавычках/);assert.throws(()=>parse("design Demo { state '' {} }"),/Имя состояния должно быть непустым/);assert.throws(()=>parse("design Demo { state 'a' { Text { text: 'x'; } } }"),/Дизайн-key должен быть непустой строкой/);});
test('validateDesign checks every state against the component',()=>{const c=parse("component Demo { Text { key: 'status'; } }");validateDesign(c,parse("design Demo { state 'a' { Text { key: 'status'; text: 'x'; } } }"));assert.throws(()=>validateDesign(c,parse("design Demo { Text { key: 'status'; } state 'a' { Text { key: 'ghost'; } } }")),/Неизвестный дизайн-key ghost/);assert.throws(()=>validateDesign(c,parse("design Demo { state 'a' { Button { key: 'status'; } } }")),/Тип дизайн-key status: ожидался Text, получен Button/);});

test('typed component contract keeps defaults, enum symbols and source ranges',()=>{
  const source=`component Notice {
    enum Tone { Accent, Danger, }
    required prop title: String;
    prop compact: Bool = false;
    prop tone: Tone = Tone.Accent;
    prop subtitle: String?;
    event selected(id: String, index: Int);
    Rectangle { text: props.title; }
  }`;
  const p=parse(source);
  assert.deepEqual(p.propDefinitions,{title:{type:'String',required:true},compact:{type:'Bool',required:false},tone:{type:'Tone',required:false},subtitle:{type:'String?',required:false}});
  assert.deepEqual(p.defaults,{compact:false,tone:{expr:'Tone.Accent'},subtitle:null});
  assert.deepEqual(p.enums,{Tone:['Accent','Danger']});
  assert.deepEqual(p.eventDefinitions.selected,[{name:'id',type:'String'},{name:'index',type:'Int'}]);
  const range=p.defaultRanges.title;assert.equal(source.slice(range.from,range.to),'required prop title: String;');
});
test('grouped matches and explicit forwarding preserve property source ranges',()=>{
  const source=`component Notice {
    match props.tone { Tone.Danger => { color: #ff0000; icon: 'error.svg'; } _ => { color: #ffffff; icon: ''; }; }
    Text { forward props { text, color }; color: #222222; }
  }`;
  const p=parse(source),branch=p.matches[0].branches[0];
  assert.equal(p.matches[0].match.expr,'props.tone');assert.equal(branch.pattern.expr,'Tone.Danger');assert.equal(branch.props.icon,'error.svg');
  assert.equal(source.slice(branch.propertyRanges.icon.from,branch.propertyRanges.icon.to),"'error.svg'");
  assert.deepEqual(p.nodes[0].forward,['text','color']);assert.equal(p.nodes[0].props.color.expr,'#222222');
});
test('conditions and keyed lists preserve event arguments and both alternative branches',()=>{
  const p=parse(`component List { Frame {
    if state.loading { Skeleton {} } else if state.failed { Text { text: 'Error'; } } else { Text { text: 'Ready'; } }
    for item in state.items key item.id {
      Button { key: 'row'; text: item.title; clicked -> events.selected(item.id, item.index + 1); }
      TextInput { value <-> item.name; }
    } empty { Text { text: 'Empty'; } }
    for other in state.other key other.id { Button { key: 'row'; clicked -> state.open(other.id); } }
  } }`);
  const [condition,list]=p.nodes[0].children;
  assert.equal(condition.type,'If');assert.equal(condition.elseChildren[0].condition.expr,'state.failed');
  assert.equal(list.type,'For');assert.equal(list.item,'item');assert.equal(list.key.expr,'item.id');assert.equal(list.emptyChildren[0].props.text,'Empty');
  assert.equal(list.children[0].events.clicked,'events.selected');assert.equal(list.children[0].eventArgs.clicked[0].expr,'item.id');assert.equal(list.children[1].bindings.value,'item.name');
});
test('declaration and control-flow mistakes produce diagnostics',()=>{
  for(const body of [
    'prop title: String;',
    "required prop title: String = 'x';",
    'prop a: Bool = false; prop a: Bool = true;',
    'enum Tone { Accent, Accent }',
    'event click(id: String, id: Int);',
    'Text { prop a: Bool = false; }',
    'Text { forward props { text, text }; }',
    'match true { true => { Text {} } }',
    'match true { true => { text: 1; } true => { text: 2; } }',
    'for state in state.items key state.id { Text {} }',
    'for item in state.items { Text {} }',
    'If {}',
  ])assert.throws(()=>parse(`component Demo { ${body} }`),body);
});
test('property expressions retain source ranges and both quote styles interpolate equally',()=>{
  const source=`component Demo { Text { text: 'Найдено: \${state.count + 1}'; disabled: state.busy || !state.valid; } }`;
  const p=parse(source),node=p.nodes[0],range=node.propertyRanges.disabled;
  assert.equal(source.slice(range.from,range.to),'state.busy || !state.valid');
  assert.equal(resolve(node.props.text,{count:2}),'Найдено: 3');
  assert.equal(resolve(node.props.disabled,{busy:false,valid:true}),false);
  assert.equal(resolve(parse(source.replaceAll("'",'"')).nodes[0].props.text,{count:2}),'Найдено: 3');
});

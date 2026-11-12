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

test('lintDocument narrows nullable types across control flow and links cross-document modules',()=>{
 const themeModule=`
  enum Tone { Accent, Danger, Muted }
  struct PriceTag {
   #[range(1, 100)]
   amount: Number;
   #[pattern('^[A-Z]{3}$')]
   currency: String;
  }
  tokens {
   spacing.md: 12px;
   palette.fg: #112233;
  }
  i18n {
   empty_title: 'Нет данных';
  }
  style CardSurface {
   padding: 8px;
  }
 `;
 const baseCardModule=`
  component BaseCard {
   required prop title: String;
   #[range(0, 1)]
   prop opacityLevel: Number = 1;
   prop tone: Tone = Tone.Accent;
   event select(id: Int);
   style BasePad {
    gap: 4px;
   }
   Text { text: props.title; }
  }
 `;
 const validScreen=`
  use './theme.forma';
  use './BaseCard.forma';

  component ProductCard : BaseCard {
   prop subtitle: String? = null;
   prop price: PriceTag = { amount: 25, currency: 'USD' };

   Column {
    apply CardSurface, BasePad;
    gap: tokens.spacing.md;

    if props.subtitle != null && !isEmpty(props.subtitle) {
     BaseCard {
      title: trim(props.subtitle);
      opacityLevel: 0.75;
      tone: Tone.Danger;
     }
    } else {
     BaseCard {
      title: i18n.empty_title;
     }
    }

    Text {
     color: tokens.palette.fg;
     text: props.subtitle != null ? upper(props.subtitle) : '\${props.price.currency} \${props.price.amount}';
    }
   }
  }
 `;
 const context={
  modules:{
   './theme.forma':themeModule,
   './BaseCard.forma':baseCardModule,
  },
 };
 assert.deepEqual(lintDocument(validScreen,context),[]);

 const invalidScreen=`
  use './theme.forma';
  use './BaseCard.forma';

  component BrokenScreen {
   state tone: Tone = Tone.Accent;
   Column {
    apply MissingStyle;
    BaseCard {
     opacityLevel: 2;
    }
    Text {
     text: match state.tone {
      Tone.Accent => 'accent';
     };
    }
   }
  }
 `;
 const codes=lintDocument(invalidScreen,context).map((d)=>d.code);
 assert.ok(codes.includes('unknown-style'),`expected unknown-style in ${codes}`);
 assert.ok(codes.includes('missing-required-prop'),`expected missing-required-prop in ${codes}`);
 assert.ok(codes.includes('out-of-range'),`expected out-of-range in ${codes}`);
 assert.ok(codes.includes('non-exhaustive-match'),`expected non-exhaustive-match in ${codes}`);
});

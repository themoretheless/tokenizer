#!/usr/bin/env python3
"""Generate highlight-only language plugin crates from a table."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CRATES = ROOT / "crates"
VERSION = "0.4.0"

# id, display, aliases, extensions, mimes, mode
# mode: clike | markup_html | markup_xml | css | yaml | toml | markdown | shell | sql
LANGS = [
    # Wave 1
    ("xml", "XML", [], [".xml", ".xsl", ".svg"], ["application/xml", "text/xml"], "markup_xml"),
    ("html", "HTML", [], [".html", ".htm"], ["text/html"], "markup_html"),
    ("css", "CSS", [], [".css"], ["text/css"], "css"),
    ("yaml", "YAML", ["yml"], [".yaml", ".yml"], ["application/yaml", "text/yaml"], "yaml"),
    ("toml", "TOML", [], [".toml"], ["application/toml"], "toml"),
    ("markdown", "Markdown", ["md"], [".md", ".markdown"], ["text/markdown"], "markdown"),
    # Wave 2
    ("sql", "SQL", [], [".sql"], ["application/sql"], "sql"),
    ("mongo", "MongoDB", ["mongodb"], [".mongodb", ".mongo"], [], "mongo"),
    ("bash", "Bash", ["shell", "sh", "zsh"], [".sh", ".bash", ".zsh"], ["application/x-sh"], "shell"),
    ("powershell", "PowerShell", ["ps1"], [".ps1", ".psm1"], [], "powershell"),
    # Wave 3
    ("javascript", "JavaScript", ["js", "jsx"], [".js", ".mjs", ".cjs", ".jsx"], ["text/javascript"], "javascript"),
    ("typescript", "TypeScript", ["ts", "tsx"], [".ts", ".tsx", ".mts", ".cts"], ["text/typescript"], "typescript"),
    # Wave 4
    ("python", "Python", ["py"], [".py", ".pyi"], ["text/x-python"], "python"),
    ("java", "Java", [], [".java"], ["text/x-java-source"], "java"),
    ("csharp", "C#", ["cs"], [".cs"], ["text/x-csharp"], "csharp"),
    ("go", "Go", ["golang"], [".go"], ["text/x-go"], "go"),
    ("php", "PHP", [], [".php"], ["application/x-httpd-php"], "php"),
    ("ruby", "Ruby", ["rb"], [".rb"], ["text/x-ruby"], "ruby"),
    # Wave 5
    ("c", "C", [], [".c", ".h"], ["text/x-c"], "c"),
    ("cpp", "C++", ["c++", "cxx"], [".cpp", ".cc", ".cxx", ".hpp", ".hh"], ["text/x-c++"], "cpp"),
    ("rust", "Rust", ["rs"], [".rs"], ["text/x-rust"], "rust"),
    ("kotlin", "Kotlin", ["kt"], [".kt", ".kts"], ["text/x-kotlin"], "kotlin"),
    ("swift", "Swift", [], [".swift"], ["text/x-swift"], "swift"),
    ("dart", "Dart", [], [".dart"], ["application/dart"], "dart"),
    ("r", "R", [], [".r", ".R"], ["text/x-r"], "r"),
    # TIOBE / industry top gaps (fullkit)
    ("visualbasic", "Visual Basic", ["vb", "vbnet"], [".vb", ".bas", ".vbs"], ["text/x-vb"], "visualbasic"),
    ("fortran", "Fortran", ["f90", "f95"], [".f", ".f90", ".f95", ".for"], ["text/x-fortran"], "fortran"),
    ("matlab", "MATLAB", ["octave"], [".m"], ["text/x-matlab"], "matlab"),
    ("delphi", "Delphi", ["pascal", "objectpascal"], [".pas", ".dpr", ".pp"], ["text/x-pascal"], "delphi"),
    ("scala", "Scala", [], [".scala", ".sc"], ["text/x-scala"], "scala"),
    ("lua", "Lua", [], [".lua"], ["text/x-lua"], "lua"),
    ("perl", "Perl", ["pl"], [".pl", ".pm", ".t"], ["text/x-perl"], "perl"),
    ("objectivec", "Objective-C", ["objc", "objective-c"], [".m", ".mm", ".h"], ["text/x-objective-c"], "objectivec"),
    ("julia", "Julia", ["jl"], [".jl"], ["text/x-julia"], "julia"),
    ("assembly", "Assembly", ["asm", "nasm"], [".asm", ".s", ".S"], ["text/x-asm"], "assembly"),
    # Wave 7 / next20 (TIOBE + GitHub next tier)
    ("groovy", "Groovy", [], [".groovy", ".gvy", ".gy", ".gsh"], ["text/x-groovy"], "groovy"),
    ("haskell", "Haskell", ["hs"], [".hs", ".lhs"], ["text/x-haskell"], "haskell"),
    ("elixir", "Elixir", ["ex"], [".ex", ".exs"], ["text/x-elixir"], "elixir"),
    ("erlang", "Erlang", ["erl"], [".erl", ".hrl"], ["text/x-erlang"], "erlang"),
    ("clojure", "Clojure", ["clj"], [".clj", ".cljs", ".cljc", ".edn"], ["text/x-clojure"], "clojure"),
    ("fsharp", "F#", ["fs", "f#"], [".fs", ".fsi", ".fsx", ".fsscript"], ["text/x-fsharp"], "fsharp"),
    ("ocaml", "OCaml", ["ml"], [".ml", ".mli"], ["text/x-ocaml"], "ocaml"),
    ("lisp", "Common Lisp", ["commonlisp", "cl"], [".lisp", ".lsp", ".cl", ".asd"], ["text/x-common-lisp"], "lisp"),
    ("scheme", "Scheme", [], [".scm", ".ss", ".rkt"], ["text/x-scheme"], "scheme"),
    ("solidity", "Solidity", ["sol"], [".sol"], ["text/x-solidity"], "solidity"),
    ("zig", "Zig", [], [".zig"], ["text/x-zig"], "zig"),
    ("nim", "Nim", [], [".nim", ".nims"], ["text/x-nim"], "nim"),
    ("dlang", "D", ["d"], [".d", ".di"], ["text/x-d"], "dlang"),
    ("cobol", "COBOL", [], [".cob", ".cbl", ".cpy"], ["text/x-cobol"], "cobol"),
    ("ada", "Ada", [], [".adb", ".ads", ".ada"], ["text/x-ada"], "ada"),
    ("prolog", "Prolog", [], [".pl", ".pro", ".P"], ["text/x-prolog"], "prolog"),
    ("abap", "ABAP", [], [".abap", ".ab4"], ["text/x-abap"], "abap"),
    ("vhdl", "VHDL", [], [".vhd", ".vhdl"], ["text/x-vhdl"], "vhdl"),
    ("verilog", "Verilog", ["systemverilog", "sv"], [".v", ".vh", ".sv", ".svh"], ["text/x-verilog"], "verilog"),
    ("graphql", "GraphQL", ["gql"], [".graphql", ".gql"], ["application/graphql"], "graphql"),
]

KEYWORDS = {
    "sql": "select insert update delete from where join left right inner outer group order by as and or not null true false create table index view into values set limit offset having distinct union all case when then else end on using",
    "mongo": "find findOne insert update delete aggregate match project group sort limit skip lookup unwind",
    "shell": "if then else elif fi for while do done case esac function in return exit export local readonly declare",
    "powershell": "if else elseif foreach for while function param begin process end switch return throw try catch finally filter",
    "javascript": "break case catch class const continue debugger default delete do else export extends finally for function if import in instanceof let new return static super switch this throw try typeof var void while with yield async await of",
    "typescript": "break case catch class const continue debugger default delete do else export extends finally for function if import in instanceof let new return static super switch this throw try typeof var void while with yield async await of type interface enum namespace declare abstract implements private protected public readonly as satisfies",
    "python": "False None True and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield match case",
    "java": "abstract assert boolean break byte case catch char class const continue default do double else enum extends final finally float for goto if implements import instanceof int interface long native new package private protected public return short static strictfp super switch synchronized this throw throws transient try void volatile while var record sealed permits yield",
    "csharp": "abstract as base bool break byte case catch char checked class const continue decimal default delegate do double else enum event explicit extern false finally fixed float for foreach goto if implicit in int interface internal is lock long namespace new null object operator out override params private protected public readonly ref return sbyte sealed short sizeof stackalloc static string struct switch this throw true try typeof uint ulong unchecked unsafe ushort using virtual void volatile while async await record required",
    "go": "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var true false nil iota",
    "php": "abstract and array as break callable case catch class clone const continue declare default do echo else elseif empty enddeclare endfor endforeach endif endswitch endwhile extends final finally for foreach function global goto if implements include include_once instanceof insteadof interface isset list match namespace new or print private protected public readonly require require_once return static switch throw trait try unset use var while xor yield true false null",
    "ruby": "BEGIN END alias and begin break case class def defined do else elsif end ensure false for if in module next nil not or redo rescue retry return self super then true undef unless until when while yield",
    "c": "auto break case char const continue default do double else enum extern float for goto if inline int long register restrict return short signed sizeof static struct switch typedef union unsigned void volatile while _Bool _Complex _Imaginary",
    "cpp": "alignas alignof and and_eq asm auto bitand bitor bool break case catch char char8_t char16_t char32_t class compl concept const consteval constexpr constinit const_cast continue co_await co_return co_yield decltype default delete do double dynamic_cast else enum explicit export extern false float for friend goto if inline int long mutable namespace new noexcept not not_eq nullptr operator or or_eq private protected public register reinterpret_cast requires return short signed sizeof static static_assert static_cast struct switch template this thread_local throw true try typedef typeid typename union unsigned using virtual void volatile wchar_t while xor xor_eq",
    "rust": "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while abstract become box do final macro override priv typeof unsized virtual yield try",
    "kotlin": "as break class continue do else false for fun if in interface is null object package return super this throw true try typealias typeof val var when while by catch constructor delegate dynamic field file finally get import init param property receiver set setparam where actual abstract annotation companion const crossinline data enum expect external final infix inline inner internal lateinit noinline open operator out override private protected public reified sealed suspend tailrec vararg",
    "swift": "associatedtype class deinit enum extension filepath func import init inout internal let operator private protocol public static struct subscript typealias var break case continue default defer do else fallthrough for guard if in repeat return switch where while as Any catch false is nil rethrows super self Self throw throws true try",
    "dart": "abstract as assert async await break case catch class const continue covariant default deferred do dynamic else enum export extends extension external factory false final finally for Function get hide if implements import in interface is late library mixin new null on operator part required rethrow return set show static super switch sync this throw true try typedef var void while with yield",
    "r": "if else repeat while function for in next break TRUE FALSE NULL NA Inf NaN",
    "yaml": "true false null yes no on off",
    "toml": "true false",
    "css": "important from to and or not only",
    "markdown": "TODO",
    "visualbasic": "AddHandler AddressOf Alias And AndAlso As Boolean ByRef Byte ByVal Call Case Catch CBool CByte CChar CDate CDbl CDec Char CInt Class CLng CObj Const Continue CSByte CShort CSng CStr CType CUInt CULng CUShort Date Decimal Declare Default Delegate Dim DirectCast Do Double Each Else ElseIf End Enum Erase Error Event Exit False Finally For Friend Function Get GetType GetXMLNamespace Global GoSub GoTo Handles If Implements Imports In Inherits Integer Interface Is IsNot Let Lib Like Long Loop Me Mod Module MustInherit MustOverride MyBase MyClass Namespace Narrowing New Next Not Nothing NotInheritable NotOverridable Object Of On Operator Option Optional Or OrElse Overloads Overridable Overrides ParamArray Partial Private Property Protected Public RaiseEvent ReadOnly ReDim REM RemoveHandler Resume Return SByte Select Set Shadows Shared Short Single Static Step Stop String Structure Sub SyncLock Then Throw To True Try TryCast TypeOf UInteger ULong UShort Using Variant When While Widening With WithEvents WriteOnly Xor Await Async Iterator Yield",
    "fortran": "program end program module end module subroutine end subroutine function end function if then else elseif endif do enddo while call return integer real double precision complex character logical parameter common data dimension equivalence external intrinsic save allocate deallocate pointer target nullify associate select case case default where elsewhere forall pure elemental recursive contains use only public private protected abstract type class extends import interface operator assignment bind value intent optional allocatable contiguous pure elemental recursive result",
    "matlab": "if else elseif end for while break continue switch case otherwise function return global persistent try catch classdef properties methods events enumeration parfor spmd true false",
    "delphi": "and array as asm begin case class const constructor destructor div do downto else end except exports file finalization finally for function goto if implementation in inherited initialization inline interface is label library mod nil not object of or packed procedure program property raise record repeat resourcestring set shl shr string then threadvar to try type unit until uses var while with xor absolute abstract assembler automated cdecl contains default delayed deprecated dispid dynamic export external far forward helper implements index message name near nodefault overload override packed pascal platform private protected public published read readonly register reintroduce requires resident safecall sealed stdcall stored strict varargs virtual write writeonly",
    "scala": "abstract case catch class def do else extends false final finally for forSome if implicit import lazy match new null object override package private protected return sealed super this throw trait true try type val var while with yield",
    "lua": "and break do else elseif end false for function goto if in local nil not or repeat return then true until while",
    "perl": "if elsif else unless while until for foreach given when default continue last next redo goto sub package use no require my our local state return do eval bless ref defined undef die warn print printf say open close read write seek tell binmode sysopen sysread syswrite pipe fork wait exec system chomp chop join split map grep sort reverse keys values each push pop shift unshift splice exists delete",
    "objectivec": "if else switch case default while for do break continue return goto sizeof typedef self super nil Nil YES NO id Class SEL IMP BOOL void int char float double long short signed unsigned const static extern volatile register inline restrict auto _Bool _Complex _Imaginary @interface @implementation @protocol @end @property @synthesize @dynamic @selector @encode @defs @class @try @catch @finally @throw @synchronized @autoreleasepool @import strong weak copy assign nonatomic atomic readonly readwrite retain",
    "julia": "baremodule begin break catch const continue do else elseif end export false finally for function global if import in isa let local macro module quote return struct true try using while where abstract primitive type mutable",
    "assembly": "section global extern db dw dd dq resb resw resd resq equ times bits use16 use32 use64 org mov add sub mul div push pop call ret jmp je jne jl jg jle jge ja jb jae jbe jz jnz loop cmp test and or xor not shl shr lea nop int syscall",
    "groovy": "as assert break case catch class const continue def default do else enum extends false finally for goto if implements import in instanceof interface new null package return super switch this throw throws trait true try while yield abstract final private protected public static synchronized volatile strictfp native transient",
    "haskell": "case class data default deriving do else if import in infix infixl infixr instance let module newtype of then type where foreign mdo proc rec qualified as hiding",
    "elixir": "after alias and case catch cond def defdelegate defexception defguard defguardp defimpl defmacro defmacrop defmodule defoverridable defp defprotocol defstruct destructure else end false fn for if import in nil not or quote raise receive require rescue super then true try unless unquote unquote_splicing use when with",
    "erlang": "after and andalso band begin bnot bor bsl bsr bxor case catch cond div end fun if let not of or orelse query receive rem try when xor true false",
    "clojure": "def defn defn- defmacro defmulti defmethod defprotocol defrecord deftype defstruct ns if do let loop recur fn quote var defonce declare import require use refer alias in-ns if-not when when-not when-let if-let cond case try catch finally throw and or not true false nil",
    "fsharp": "abstract and as assert base begin class default delegate do done downcast downto elif else end exception extern false finally for fun function global if in inherit inline interface internal lazy let match member module mutable namespace new not null of open or override private public rec return static struct then to true try type upcast use val void when while with yield async await",
    "ocaml": "and as assert asr begin class constraint do done downto else end exception external false for fun function functor if in include inherit initializer land lazy let lor lsl lsr lxor match method mod module mutable new nonrec object of open or private rec sig struct then to true try type val virtual when while with",
    "lisp": "defun defmacro defvar defparameter defconstant defclass defmethod defgeneric lambda if when unless cond case ecase typecase let let star flet labels progn prog1 prog2 block return return-from tagbody go catch throw unwind-protect multiple-value-bind values quote function setq setf push pop and or not t nil loop do dolist dotimes",
    "scheme": "define define-syntax lambda if cond case and or not begin let let star letrec do delay force quote quasiquote unquote unquote-splicing set! car cdr cons list pair? null? eq? eqv? equal? #t #f else",
    "solidity": "pragma contract interface library abstract is using for struct enum mapping function modifier event error constructor fallback receive if else for while do break continue return throw emit try catch public private internal external view pure payable virtual override immutable constant anonymous indexed memory storage calldata true false address bool string bytes uint int",
    "zig": "addrspace align allowzero and anyframe anytype asm async await break callconv catch comptime const continue defer else enum errdefer error export extern fn for if inline linksection noalias nosuspend opaque or orelse packed pub resume return struct suspend switch test threadlocal try union unreachable usingnamespace var volatile while true false null undefined",
    "nim": "addr and as asm bind block break case cast concept const continue converter defer discard distinct div do elif else end enum except export finally for from func if import in include interface is isnot iterator let macro method mixin mod nil not notin object of or out proc ptr raise ref return shl shr static template try tuple type using var when while xor yield true false",
    "dlang": "abstract alias align asm assert auto body bool break byte case cast catch cdouble cent cfloat char class const continue creal dchar debug default delegate delete deprecated do double else enum export extern false final finally float for foreach foreach_reverse function goto idouble if ifloat immutable import in inout int interface invariant ireal is lazy long macro mixin module new nothrow null out override package pragma private protected public pure real ref return scope shared short static struct super switch synchronized template this throw true try typedef typeid typeof ubyte ucent uint ulong union unittest ushort version void volatile wchar while with",
    "cobol": "ACCEPT ADD ALTER CALL CANCEL CLOSE COMPUTE CONTINUE DELETE DISPLAY DIVIDE ELSE END EVALUATE EXIT GO GOBACK IF INITIALIZE INSPECT MERGE MOVE MULTIPLY OPEN PERFORM READ RELEASE RETURN REWRITE SEARCH SET SORT START STOP STRING SUBTRACT UNSTRING WRITE WORKING-STORAGE SECTION DIVISION PROGRAM-ID IDENTIFICATION ENVIRONMENT DATA PROCEDURE AUTHOR DATE-WRITTEN",
    "ada": "abort abs abstract accept access aliased all and array at begin body case constant declare delay delta digits do else elsif end entry exception exit for function generic goto if in interface is limited loop mod new not null of or others out overriding package pragma private procedure protected raise range record rem renames requeue return reverse select separate some subtype synchronized tagged task terminate then type until use when while with xor true false",
    "prolog": "is mod rem div not fail true false once repeat forall between member append length reverse sort keysort findall bagof setof assert asserta assertz retract retractall clause abolish dynamic multifile discontiguous public op current_predicate",
    "abap": "DATA TYPES CONSTANTS TABLES PARAMETERS SELECT-OPTIONS RANGES FIELD-SYMBOLS IF ELSE ELSEIF ENDIF CASE WHEN ENDCASE DO ENDDO WHILE ENDWHILE LOOP ENDLOOP EXIT CONTINUE CHECK RETURN FORM ENDFORM PERFORM FUNCTION ENDFUNCTION MODULE ENDMODULE CLASS ENDCLASS METHOD ENDMETHOD INTERFACE ENDINTERFACE TRY CATCH CLEANUP ENDTRY RAISE WRITE MOVE CLEAR REFRESH APPEND INSERT DELETE MODIFY READ SELECT ENDSELECT UPDATE INSERT FROM INTO WHERE AND OR NOT EQ NE LT GT LE GE",
    "vhdl": "abs access after alias all and architecture array assert attribute begin block body buffer bus case component configuration constant disconnect downto else elsif end entity exit file for function generate generic group guarded if impure in inertial inout is label library linkage literal loop map mod nand new next nor not null of on open or others out package port postponed procedure process pure range record register reject rem report return rol ror select severity shared signal sla sll sra srl subtype then to transport type unaffected units until use variable wait when while with xnor xor true false",
    "verilog": "always always_comb always_ff always_latch and assign automatic begin buf bufif0 bufif1 case casex casez cell cmos config deassign default defparam design disable edge else end endcase endconfig endfunction endgenerate endmodule endprimitive endspecify endtable endtask event for force forever fork function generate genvar highz0 highz1 if ifnone initial inout input integer join large macromodule medium module nand negedge nmos nor noshowcancelled not notif0 notif1 or output parameter pmos posedge primitive pull0 pull1 pullup pulldown pulsestyle_ondetect pulsestyle_onevent rcmos real realtime reg release repeat rnmos rpmos rtran rtranif0 rtranif1 scalared showcancelled signed small specify specparam strong0 strong1 supply0 supply1 table task time tran tranif0 tranif1 tri tri0 tri1 triand trior trireg unsigned use uwire vectored wait wand weak0 weak1 while wire wor xnor xor",
    "graphql": "query mutation subscription schema type interface union enum input extend implements fragment on scalar true false null directive repeatable",
}

TYPES = {
    "typescript": "string number boolean any void never unknown object symbol bigint",
    "java": "String int Integer long Long boolean Boolean void double Double float Float char Character byte Byte short Short",
    "csharp": "string int long bool void double float decimal object byte short uint ulong char",
    "c": "int char void long short float double size_t uint8_t uint16_t uint32_t uint64_t int8_t int16_t int32_t int64_t",
    "cpp": "int char void long short float double size_t bool string vector map set optional unique_ptr shared_ptr",
    "rust": "i8 i16 i32 i64 u8 u16 u32 u64 isize usize f32 f64 bool char str String Vec Option Result",
    "kotlin": "Int Long Boolean String Double Float Char Byte Short Unit Any",
    "swift": "Int Double Float Bool String Character Array Dictionary Set Optional",
    "dart": "int double num bool String List Map Set Object void",
    "go": "int int8 int16 int32 int64 uint string bool byte rune float32 float64 error",
    "php": "int float string bool array object void mixed",
    "solidity": "address bool string bytes byte int uint int8 int16 int32 int64 int128 int256 uint8 uint16 uint32 uint64 uint128 uint256 bytes1 bytes32 fixed ufixed",
    "zig": "i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 isize usize f16 f32 f64 f128 bool void noreturn type anyerror anyframe anyopaque comptime_int comptime_float",
    "nim": "int int8 int16 int32 int64 uint uint8 uint16 uint32 uint64 float float32 float64 bool char string cstring pointer",
    "dlang": "bool byte ubyte short ushort int uint long ulong cent ucent float double real char wchar dchar void",
    "fsharp": "int int16 int32 int64 uint uint16 uint32 uint64 float float32 double decimal bool byte sbyte char string unit option list array seq",
    "ocaml": "int float bool char string unit list option array ref",
    "ada": "Integer Natural Positive Float Boolean Character String Duration",
    "vhdl": "std_logic std_logic_vector signed unsigned integer natural positive boolean bit bit_vector real time",
    "verilog": "wire reg logic integer real time realtime",
    "graphql": "Int Float String Boolean ID",
}

def kw_list(s: str) -> str:
    parts = [p for p in s.split() if p]
    return ",\n        ".join(f'"{p}"' for p in parts)

def rust_str_array(items) -> str:
    if not items:
        return "&[]"
    inner = ", ".join(f'"{x}"' for x in items)
    return f"&[{inner}]"

def tokenize_body(mode: str, lid: str) -> str:
    if mode == "markup_html":
        return "    highlight_markup(source, true)"
    if mode == "markup_xml":
        return "    highlight_markup(source, false)"
    if mode == "css":
        return "    highlight_css(source)"
    if mode == "yaml":
        return "    highlight_yaml(source)"
    if mode == "toml":
        return "    highlight_toml(source)"
    if mode == "markdown":
        return "    highlight_markdown(source)"

    kws = KEYWORDS.get(lid, KEYWORDS.get(mode, ""))
    types = TYPES.get(lid, "")
    line = 'Some("//")'
    block = 'Some(("/*", "*/"))'
    hash_c = "false"
    strings = "StringStyle::CStyle"
    ident = "IdentStyle::Ascii"

    if mode in ("shell", "bash"):
        line = "None"
        block = "None"
        hash_c = "true"
        strings = "StringStyle::Shell"
    elif mode == "powershell":
        line = 'Some("#")'
        block = 'Some(("<#", "#>"))'
        hash_c = "false"
        strings = "StringStyle::CStyle"
        # # is also line comment via line_comment
        line = 'Some("#")'
    elif mode == "python":
        line = "None"
        block = "None"
        hash_c = "true"
        strings = "StringStyle::Python"
    elif mode == "ruby":
        line = "None"
        block = "None"
        hash_c = "true"
        strings = "StringStyle::CStyle"
    elif mode == "sql":
        line = 'Some("--")'
        block = 'Some(("/*", "*/"))'
    elif mode == "mongo":
        line = 'Some("//")'
        block = 'Some(("/*", "*/"))'
    elif mode == "php":
        ident = "IdentStyle::AsciiDollar"
    elif mode == "javascript" or mode == "typescript":
        ident = "IdentStyle::AsciiDollar"
    elif mode == "r":
        line = 'Some("#")'
        block = "None"
        hash_c = "false"

    return f"""    let profile = CLikeProfile {{
        keywords: &[
        {kw_list(kws)}
        ],
        types: &[
        {kw_list(types)}
        ],
        builtins: &[],
        line_comment: {line},
        block_comment: {block},
        hash_line_comment: {hash_c},
        strings: {strings},
        ident_continue: {ident},
    }};
    highlight_c_like(source, &profile)"""

def lang_const(lid: str) -> str:
    mapping = {
        "xml": "XML", "html": "HTML", "css": "CSS", "yaml": "YAML", "toml": "TOML",
        "markdown": "MARKDOWN", "sql": "SQL", "mongo": "MONGO", "bash": "BASH",
        "powershell": "POWERSHELL", "javascript": "JAVASCRIPT", "typescript": "TYPESCRIPT",
        "python": "PYTHON", "java": "JAVA", "csharp": "CSHARP", "go": "GO", "php": "PHP",
        "ruby": "RUBY", "c": "C", "cpp": "CPP", "rust": "RUST", "kotlin": "KOTLIN",
        "swift": "SWIFT", "dart": "DART", "r": "R",
        "visualbasic": "VISUALBASIC", "fortran": "FORTRAN", "matlab": "MATLAB",
        "delphi": "DELPHI", "scala": "SCALA", "lua": "LUA", "perl": "PERL",
        "objectivec": "OBJECTIVEC", "julia": "JULIA", "assembly": "ASSEMBLY",
        "groovy": "GROOVY", "haskell": "HASKELL", "elixir": "ELIXIR", "erlang": "ERLANG",
        "clojure": "CLOJURE", "fsharp": "FSHARP", "ocaml": "OCAML", "lisp": "LISP",
        "scheme": "SCHEME", "solidity": "SOLIDITY", "zig": "ZIG", "nim": "NIM",
        "dlang": "DLANG", "cobol": "COBOL", "ada": "ADA", "prolog": "PROLOG",
        "abap": "ABAP", "vhdl": "VHDL", "verilog": "VERILOG", "graphql": "GRAPHQL",
    }
    return mapping[lid]

FULL_MODES = {
    "sql", "mongo", "shell", "powershell", "javascript", "typescript", "python",
    "java", "csharp", "go", "php", "ruby", "c", "cpp", "rust", "kotlin", "swift",
    "dart", "r", "bash", "yaml", "toml", "css", "markdown",
    "visualbasic", "fortran", "matlab", "delphi", "scala", "lua", "perl",
    "objectivec", "julia", "assembly",
    "groovy", "haskell", "elixir", "erlang", "clojure", "fsharp", "ocaml", "lisp",
    "scheme", "solidity", "zig", "nim", "dlang", "cobol", "ada", "prolog",
    "abap", "vhdl", "verilog", "graphql",
}

def full_profile_body(mode: str, lid: str) -> str:
    kws = KEYWORDS.get(lid, KEYWORDS.get(mode, ""))
    types = TYPES.get(lid, "")
    line = 'Some("//")'
    block = 'Some(("/*", "*/"))'
    hash_c = "false"
    dollar = "false"
    triple = "false"
    soft = "false"
    if mode in ("shell", "bash"):
        line = "None"
        block = "None"
        hash_c = "true"
    elif mode == "powershell":
        line = 'Some("#")'
        block = 'Some(("<#", "#>"))'
    elif mode == "python":
        line = "None"
        block = "None"
        hash_c = "true"
        triple = "true"
        soft = "true"
    elif mode == "ruby":
        line = "None"
        block = "None"
        hash_c = "true"
    elif mode == "sql":
        line = 'Some("--")'
    elif mode in ("javascript", "typescript", "php"):
        dollar = "true"
    elif mode == "r":
        line = 'Some("#")'
        block = "None"
    elif mode in ("yaml", "toml", "markdown"):
        line = "None"
        block = "None"
        hash_c = "true"
    elif mode == "css":
        line = "None"
        block = 'Some(("/*", "*/"))'
        dollar = "true"
    elif mode == "visualbasic":
        line = 'Some("\'")'
        block = "None"
    elif mode == "fortran":
        line = 'Some("!")'
        block = "None"
    elif mode == "matlab":
        line = 'Some("%")'
        block = 'Some(("%{", "%}"))'
    elif mode == "delphi":
        line = 'Some("//")'
        block = 'Some(("{", "}"))'
    elif mode == "lua":
        line = 'Some("--")'
        block = 'Some(("--[[", "]]"))'
    elif mode == "perl":
        line = 'Some("#")'
        block = "None"
        hash_c = "false"
        dollar = "true"
    elif mode == "julia":
        line = 'Some("#")'
        block = 'Some(("#=", "=#"))'
    elif mode == "assembly":
        line = 'Some(";")'
        block = "None"
    elif mode == "scala":
        line = 'Some("//")'
        block = 'Some(("/*", "*/"))'
    elif mode == "objectivec":
        line = 'Some("//")'
        block = 'Some(("/*", "*/"))'
    elif mode == "groovy":
        line = 'Some("//")'
        block = 'Some(("/*", "*/"))'
        dollar = "true"
    elif mode == "haskell":
        line = 'Some("--")'
        block = 'Some(("{-", "-}"))'
    elif mode == "elixir":
        line = "None"
        block = "None"
        hash_c = "true"
    elif mode == "erlang":
        line = 'Some("%")'
        block = "None"
    elif mode == "clojure":
        line = 'Some(";")'
        block = "None"
    elif mode == "fsharp":
        line = 'Some("//")'
        block = 'Some(("(*", "*)"))'
    elif mode == "ocaml":
        line = "None"
        block = 'Some(("(*", "*)"))'
    elif mode in ("lisp", "scheme"):
        line = 'Some(";")'
        block = "None"
    elif mode == "solidity":
        line = 'Some("//")'
        block = 'Some(("/*", "*/"))'
    elif mode == "zig":
        line = 'Some("//")'
        block = 'Some(("/*", "*/"))'
    elif mode == "nim":
        line = 'Some("#")'
        block = 'Some(("#[", "]#"))'
    elif mode == "dlang":
        line = 'Some("//")'
        block = 'Some(("/*", "*/"))'
    elif mode == "cobol":
        line = 'Some("*>")'
        block = "None"
    elif mode == "ada":
        line = 'Some("--")'
        block = "None"
    elif mode == "prolog":
        line = 'Some("%")'
        block = 'Some(("/*", "*/"))'
    elif mode == "abap":
        line = 'Some("\\"")'
        block = "None"
    elif mode == "vhdl":
        line = 'Some("--")'
        block = "None"
    elif mode == "verilog":
        line = 'Some("//")'
        block = 'Some(("/*", "*/"))'
    elif mode == "graphql":
        line = "None"
        block = "None"
        hash_c = "true"
    return f"""    FullProfile {{
        keywords: &[{kw_list(kws)}],
        types: &[{kw_list(types)}],
        line_comment: {line},
        block_comment: {block},
        hash_line_comment: {hash_c},
        dollar_ident: {dollar},
        triple_strings: {triple},
        soft_indent_blocks: {soft},
    }}"""

def gen_crate(lid, display, aliases, extensions, mimes, mode):
    crate_dir = CRATES / f"tokenizer-{lid}"
    src = crate_dir / "src"
    src.mkdir(parents=True, exist_ok=True)

    is_full = mode in FULL_MODES or lid in FULL_MODES
    desc = (
        f"Full {display} engine (lex/parse/AST/semantic) for themoretheless-tokenizer"
        if is_full
        else f"Lossless {display} highlighter plugin for themoretheless-tokenizer"
    )
    cargo = f'''[package]
name = "themoretheless-tokenizer-{lid}"
version = "{VERSION}"
edition = "2024"
authors = ["Denis Mezhov"]
description = "{desc}"
license = "MIT OR Apache-2.0"
repository = "https://github.com/themoretheless/tokenizer"
readme = "README.md"
publish = ["crates-io"]
keywords = ["tokenizer", "lexer", "{lid}", "parser"]
categories = ["parsing", "text-processing"]

[lib]
name = "themoretheless_tokenizer_{lid.replace('-', '_')}"
path = "src/lib.rs"

[dependencies]
themoretheless-tokenizer-core = {{ path = "../tokenizer-core", version = "{VERSION}" }}

[package.metadata.docs.rs]
all-features = true
'''
    (crate_dir / "Cargo.toml").write_text(cargo)

    if is_full:
        readme = f"""# themoretheless-tokenizer-{lid}

**Full** {display} plugin: lossless lex, recovering parse, shared AST (`Module`/`Item`/`Stmt`/`Expr`),
semantic tokens, diagnostics.

Uses `themoretheless-tokenizer-core::fullkit` with a language profile.
"""
    else:
        readme = f"""# themoretheless-tokenizer-{lid}

**{display}** plugin (structured highlight; full markup AST pipeline shared via host).
"""
    (crate_dir / "README.md").write_text(readme)

    aliases_rs = rust_str_array(aliases)
    ext_rs = rust_str_array(extensions)
    mime_rs = rust_str_array(mimes)
    const = lang_const(lid)

    if is_full:
        profile = full_profile_body(mode if mode != "bash" else "shell", lid)
        lib = f'''//! Full {display} engine (lex → parse → AST → semantic).

use themoretheless_tokenizer_core::{{
    Diagnostic, FullProfile, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostTokenization, LanguageDescriptor, LanguageId, Lexed, Parse, SemanticTokenization,
    analyze_full_host, full_descriptor, lex_full, lex_to_host, parse_full, require_default_dialect,
    semantic_full,
}};

fn profile() -> FullProfile {{
{profile}
}}

/// Lossless lexer.
#[must_use]
pub fn lex(source: &str) -> Lexed {{
    lex_full(source, &profile())
}}

/// Recovering parse with borrowing AST.
#[must_use]
pub fn parse(source: &str) -> Parse<'_> {{
    parse_full(source, &profile())
}}

/// Parser-aware semantic tokens.
#[must_use]
pub fn tokenize(source: &str) -> SemanticTokenization {{
    semantic_full(&parse(source))
}}

/// Diagnostics from the full pipeline.
#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {{
    parse(source).diagnostics
}}

/// Host adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

pub static ENGINE: Host = Host;

pub static DESCRIPTOR: LanguageDescriptor = full_descriptor(
    LanguageId::{const},
    "{display}",
    {aliases_rs},
    {ext_rs},
    {mime_rs},
    env!("CARGO_PKG_VERSION"),
);

impl HostLanguage for Host {{
    fn descriptor(&self) -> &'static LanguageDescriptor {{
        &DESCRIPTOR
    }}

    fn lex(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {{
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        if opts.limits.exceeds_input_bytes(source.len()) {{
            return Err(HostError::InputTooLarge {{
                max: opts.limits.max_input_bytes,
                actual: source.len(),
            }});
        }}
        Ok(lex_to_host(lex(source)))
    }}

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {{
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        if opts.limits.exceeds_input_bytes(source.len()) {{
            return Err(HostError::InputTooLarge {{
                max: opts.limits.max_input_bytes,
                actual: source.len(),
            }});
        }}
        Ok(analyze_full_host(source, &profile()))
    }}

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {{
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        Ok(validate(source)
            .into_iter()
            .map(themoretheless_tokenizer_core::HostDiagnostic::from_diagnostic)
            .collect())
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn lossless_lex() {{
        let source = "fn main() {{ return 1; }}";
        assert!(lex(source).is_lossless(source));
    }}

    #[test]
    fn parse_smoke() {{
        let source = "function f(x) {{ return x + 1; }}";
        let p = parse(source);
        assert!(p.lexed.is_lossless(source));
        assert!(!p.module.items.is_empty() || source.is_empty());
    }}
}}
'''
    else:
        body = tokenize_body(mode, lid)
        if mode == "markup_html" or mode == "markup_xml":
            imports = """use themoretheless_tokenizer_core::{
    Diagnostic, Highlighted, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostTokenization, LanguageDescriptor, LanguageId, highlight_markup, highlight_descriptor,
    run_diagnose_host, run_highlight_host,
};"""
        elif mode == "css":
            imports = """use themoretheless_tokenizer_core::{
    Diagnostic, Highlighted, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostTokenization, LanguageDescriptor, LanguageId, highlight_css, highlight_descriptor,
    run_diagnose_host, run_highlight_host,
};"""
        elif mode == "yaml":
            imports = """use themoretheless_tokenizer_core::{
    Diagnostic, Highlighted, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostTokenization, LanguageDescriptor, LanguageId, highlight_yaml, highlight_descriptor,
    run_diagnose_host, run_highlight_host,
};"""
        elif mode == "toml":
            imports = """use themoretheless_tokenizer_core::{
    Diagnostic, Highlighted, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostTokenization, LanguageDescriptor, LanguageId, highlight_toml, highlight_descriptor,
    run_diagnose_host, run_highlight_host,
};"""
        else:
            imports = """use themoretheless_tokenizer_core::{
    Diagnostic, Highlighted, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostTokenization, LanguageDescriptor, LanguageId, highlight_markdown, highlight_descriptor,
    run_diagnose_host, run_highlight_host,
};"""
        lib = f'''//! {display} engine (markup/data pipeline).

{imports}

#[must_use]
pub fn tokenize(source: &str) -> Highlighted {{
{body}
}}

#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {{
    tokenize(source).diagnostics
}}

#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

pub static ENGINE: Host = Host;

pub static DESCRIPTOR: LanguageDescriptor = highlight_descriptor(
    LanguageId::{const},
    "{display}",
    {aliases_rs},
    {ext_rs},
    {mime_rs},
    env!("CARGO_PKG_VERSION"),
);

impl HostLanguage for Host {{
    fn descriptor(&self) -> &'static LanguageDescriptor {{
        &DESCRIPTOR
    }}

    fn lex(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {{
        run_highlight_host(&DESCRIPTOR, source, opts, tokenize)
    }}

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {{
        self.lex(source, opts)
    }}

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {{
        run_diagnose_host(&DESCRIPTOR, source, opts, tokenize)
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;
    #[test]
    fn lossless_smoke() {{
        for source in ["", "x", "<a/>", "# t"] {{
            assert!(tokenize(source).is_lossless(source));
        }}
    }}
}}
'''
    (src / "lib.rs").write_text(lib)
    print(f"generated tokenizer-{lid} ({'full' if is_full else 'markup'})")

def main():
    for row in LANGS:
        gen_crate(*row)
    print(f"done: {len(LANGS)} languages")

if __name__ == "__main__":
    main()

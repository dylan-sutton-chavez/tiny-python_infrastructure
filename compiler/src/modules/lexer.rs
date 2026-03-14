/* 
lexer.rs
  Reads raw bytes, emits tokens with start/end positions.
  No strings, no copies — offsets into the original buffer only.
*/

use logos::Logos;

#[derive(Logos)]
#[logos(skip r"[ \t\r]+")]
enum TokenType {

  /* 
  Keywords
  */

  #[token("False")] False,
  #[token("None")] None,
  #[token("True")] True,
  #[token("and")] And,
  #[token("as")] As,
  #[token("assert")] Assert,
  #[token("async")] Async,
  #[token("await")] Await,
  #[token("break")] Break,
  #[token("class")] Class,
  #[token("continue")] Continue,
  #[token("def")] Def,
  #[token("del")] Del,
  #[token("elif")] Elif,
  #[token("else")] Else,
  #[token("except")] Except,
  #[token("finally")] Finally,
  #[token("for")] For,
  #[token("from")] From,
  #[token("global")] Global,
  #[token("if")] If,
  #[token("import")] Import,
  #[token("in")] In,
  #[token("is")] Is,
  #[token("lambda")] Lambda,
  #[token("nonlocal")] Nonlocal,
  #[token("not")] Not,
  #[token("or")] Or,
  #[token("pass")] Pass,
  #[token("raise")] Raise,
  #[token("return")] Return,
  #[token("try")] Try,
  #[token("while")] While,
  #[token("with")] With,
  #[token("yield")] Yield,

  /*
  Soft keywords
  */

  #[token("case")] Case,
  #[token("match")] Match,
  #[token("type")] Type,
  #[token("_")] Underscore,

  /*
  Operators
  */

  #[token("**=")] DoubleStarEqual,
  #[token("//=")] DoubleSlashEqual,
  #[token("<<=")] LeftShiftEqual,
  #[token(">>=")] RightShiftEqual,

  #[token("!=")] NotEqual,
  #[token("%=")] PercentEqual,
  #[token("&=")] AmperEqual,
  #[token("**")] DoubleStar,
  #[token("*=")] StarEqual,
  #[token("+=")] PlusEqual,
  #[token("-=")] MinEqual,
  #[token("->")] Rarrow,
  #[token("...")] Ellipsis,
  #[token("//")] DoubleSlash,
  #[token("/=")] SlashEqual,
  #[token(":=")] ColonEqual,
  #[token("<<")] LeftShift,
  #[token("<=")] LessEqual,
  #[token("==")] EqEqual,
  #[token(">=")] GreaterEqual,
  #[token(">>")] RightShift,
  #[token("@=")] AtEqual,
  #[token("^=")] CircumflexEqual,
  #[token("|=")] VbarEqual,

  #[token("!")] Exclamation,
  #[token("%")] Percent,
  #[token("&")] Amper,
  #[token("*")] Star,
  #[token("+")] Plus,
  #[token("-")] Minus,
  #[token(".")] Dot,
  #[token("/")] Slash,
  #[token("<")] Less,
  #[token("=")] Equal,
  #[token(">")] Greater,
  #[token("@")] At,
  #[token("^")] Circumflex,
  #[token("|")] Vbar,
  #[token("~")] Tilde,

  /*
  Delimitors
  */

  #[token("(")] Lpar,
  #[token(")")] Rpar,
  #[token("[")] Lsqb,
  #[token("]")] Rsqb,
  #[token("{")] Lbrace,
  #[token("}")] Rbrace,
  #[token(",")] Comma,
  #[token(":")] Colon,
  #[token(";")] Semi,

  /*
  Token names
  */

  #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*")] Name,

  #[regex(r"[0-9]+[jJ]")]
  #[regex(r"[0-9]+\.[0-9]*([eE][+-]?[0-9]+)?[jJ]")]
  #[regex(r"\.[0-9]+([eE][+-]?[0-9]+)?[jJ]")]
  Complex,

  #[regex(r"[0-9]+\.[0-9]*([eE][+-]?[0-9]+)?")]
  #[regex(r"\.[0-9]+([eE][+-]?[0-9]+)?")]
  #[regex(r"[0-9]+[eE][+-]?[0-9]+")]
  Float,

  #[regex(r"0[xX][0-9a-fA-F][0-9a-fA-F_]*")]
  #[regex(r"0[oO][0-7][0-7_]*")]
  #[regex(r"0[bB][01][01_]*")]
  #[regex(r"[1-9][0-9_]*|0")]
  Int,

  #[regex(r#"[bBrRuU]{0,2}"""([^"\\]|\\.|"(?!""))*""""#)]
  #[regex(r#"[bBrRuU]{0,2}'''([^'\\]|\\.|'(?!''))*'''"#)]
  #[regex(r#"[bBrRuU]{0,2}"([^"\\\n]|\\.)*""#)]
  #[regex(r#"[bBrRuU]{0,2}'([^'\\\n]|\\.)*'"#)]
  String,

  FstringStart,
  FstringMiddle,
  FstringEnd,

  #[regex(r"#[^\n]*")] Comment,
    
  #[token("\n")] Newline,

  Indent,
  Dedent,

  Nl,

  Endmarker

};
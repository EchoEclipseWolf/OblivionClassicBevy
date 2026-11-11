"""Rust source tokens used by the workspace analysis and migration tools."""
import re

TOKEN = re.compile(
    r"""\s+|//[^\n]*|/\*|(?:br|cr|r)(?P<hash>\#*)".*?"(?P=hash)"""
    r"""|(?:b|c)?"(?:\\.|[^"\\])*"|b?'(?:\\(?:u\{\w+\}|x\w\w|.)|[^'\\\n])'"""
    r"""|[0-9][a-zA-Z0-9_]*|(?:r\#)?[a-zA-Z_]\w*|::|.""", re.S,
)
COMMENT = re.compile(r"/\*|\*/")


def token_spans(source):
    """Yield each Rust token and its half-open character span. Skip comments and whitespace."""
    pos = 0
    while pos < len(source):
        match = TOKEN.match(source, pos)
        word = match[0]
        end = match.end()
        if word == "/*":
            depth = 1
            for nested in COMMENT.finditer(source, end):
                depth += 1 if nested[0] == "/*" else -1
                if not depth:
                    end = nested.end()
                    break
            else:
                raise ValueError("unterminated block comment")
        elif not word.isspace() and not word.startswith("//"):
            yield word, pos, end
        pos = end


def tokens(source):
    line = 1
    pos = 0
    for word, start, _ in token_spans(source):
        line += source.count("\n", pos, start)
        yield word.removeprefix("r#"), line
        pos = start

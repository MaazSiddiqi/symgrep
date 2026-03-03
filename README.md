```bash
# The problem: grep finds lines, not meaning
$ grep -r "usePaginated" src/
src/hooks/db/usePaginatedQuery.ts:64:export default function usePaginatedQuery...
src/hooks/db/usePaginatedSearchQuery.ts:38:export default function usePaginatedSearchQuery...
src/components/admin/HackerTable.tsx:24:  } = usePaginatedSearchQuery<HackerRow>({
# Now read each file to understand the context...

# The symgrep solution: one command, semantic context
$ symgrep usePaginated ./src

src/hooks/db/usePaginatedQuery.ts:64  node_type=identifier  node_lines=[64..138]
export default function usePaginatedQuery<TItem>({
  path,
  queryKey,
  initialPageParam = 1,
}: UsePaginatedQueryOptions) {
  const { data, fetchNextPage, hasNextPage: hasMore, ...rest } = useInfiniteQuery({
    queryFn: async ({ pageParam = 1 }) => { ... },
    getNextPageParam: (lastPage) => { ... },
  });
  return { data: flattenedResults, loadMore, hasMore, totalCount, ...rest };
}

src/components/admin/HackerTable.tsx:24  node_type=identifier  node_lines=[24..26]
  } = usePaginatedSearchQuery<HackerRow>({
    path: "/backend/api/profiles/hackers/"
  });

# Full function bodies, call expressions, type definitions, and comments —
# whatever scope the AST says actually contains the symbol
```

# symgrep

Semantic symbol search for codebases

## What is this?

`symgrep` searches for a symbol across a codebase and returns each occurrence wrapped in its actual AST context — not ±N arbitrary lines, but the function, block, or expression that syntactically contains it.

It replaces the grep → read → understand loop with a single query. A coding agent or developer can immediately distinguish a definition from a call site from an import, without opening any files.

## Usage

```bash
symgrep <pattern> [path]
```

```bash
# Search in current directory
symgrep useState

# Search a specific path
symgrep usePaginated ./src/hooks

# Works on Rust too
symgrep parse_match_occurrences ./src
```

## Output Format

Each result shows:
- **File path + line number** of the match
- **`node_type`** — the tree-sitter node kind at the match site
- **`node_lines`** — the line range of the rendered context
- **The context block** — the full AST scope, with the match highlighted

```
src/hooks/db/usePaginatedQuery.ts:9  node_type=type_identifier  node_lines=[9..30]
type UsePaginatedQueryOptions = {
  path: QueriedPath;
  queryKey: string[];
  initialPageParam?: number;
};

src/hooks/db/usePaginatedSearchQuery.ts:52  node_type=identifier  node_lines=[52..55]
    usePaginatedQuery<TItem>({
      path: searchEndpoint,
      queryKey: ["search", searchEndpoint],
    });
```

The `node_type` field immediately tells you what you're looking at:

| node_type | What it means |
|-----------|---------------|
| `function_declaration` / `function_item` | Definition |
| `call_expression` | Call site |
| `type_identifier` | Type reference |
| `identifier` | Variable / import specifier |
| `string_fragment` | Inside a string literal (e.g. import path) |
| `comment` | Referenced in a doc comment |

## How It Works

symgrep pipes `ripgrep --json` output through tree-sitter. Ripgrep provides byte-accurate match offsets; tree-sitter uses those offsets to locate the exact AST leaf node. The tree is then walked upward, up to 8 ancestors, selecting the nearest meaningful scope — a function declaration, call expression, block, or type definition — capped at 200 lines.

Files are parsed once and cached, so multiple matches in the same file share a single parse pass.

## Installation

Requires [Rust](https://rustup.rs/) and [ripgrep](https://github.com/BurntSushi/ripgrep).

```bash
git clone https://github.com/MaazSiddiqi/symgrep.git
cd symgrep
cargo build --release
cp target/release/symgrep /usr/local/bin/symgrep
```

## Supported Languages

| Language | Extensions |
|----------|------------|
| TypeScript | `.ts`, `.js` |
| TSX / JSX | `.tsx`, `.jsx` |
| Rust | `.rs` |

## Why?

When I started using coding agents, I noticed they spent a lot of time in a repetitive loop: grep for a symbol, read the surrounding file, grep again for a related symbol, read another file. The information they needed was rarely the whole file — it was one function, one call site, one type definition.

symgrep was built to collapse that loop. The right context, already extracted, on the first query.

## Roadmap

- **Deduplication** — skip duplicate renders when multiple matches fall inside the same AST node
- **Node type filtering** — `--only definitions`, `--only calls`, etc.
- **More languages** — Python, Go, Java via additional tree-sitter grammars
- **Exact case mode** — opt out of the default case-insensitive search

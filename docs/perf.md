# supp perf

Control CPU and memory usage with performance modes. On large codebases (50k+ files), supp can consume significant resources during symbol indexing, call site discovery, and context analysis. Performance modes trade some accuracy or completeness for lower resource consumption.

## Usage

```bash
# Set the global performance mode (persisted to ~/.config/supp/perf)
supp perf lite

# Check the current mode
supp perf

# Override for a single command with -p / --perf
supp -p full sym handler
```

Available modes: `full` (default), `balanced`, `lite`.

The mode can also be set via the `SUPP_PERF` environment variable.

### Precedence

```
-p / --perf flag  >  SUPP_PERF env var  >  supp perf (persisted)  >  default (full)
```

## Modes at a glance

| Knob | `full` | `balanced` | `lite` |
|------|--------|------------|--------|
| Thread pool | all cores | half (min 2) | 2 |
| PageRank iterations | 15 | 8 | 5 |
| Max files (context) | 20,000 | 50,000 | 10,000 |
| Max content (context) | 50 MB | 100 MB | 30 MB |
| Call sites cap | 30 | 30 | 15 |
| Call sites early exit | no | yes | yes |
| Used-by threshold | 20 files | 10 files | disabled |

## When to use each mode

- **`full`** - Default. Best results on small-to-medium projects (under ~20k files). Uses all available CPU cores.
- **`balanced`** - Good for large projects (20k-100k files) or when you're running supp alongside other CPU-intensive work. Halves core usage and enables early exit optimizations.
- **`lite`** - Best for monorepos (100k+ files) or resource-constrained environments. Caps at 2 threads, disables expensive scans, and tightens file limits. Results may be less complete but supp stays out of your way.

## Examples

```bash
# Set lite mode globally for a huge monorepo
supp perf lite

# Check what mode you're using
supp perf

# Override for a single command
supp -p full sym handler

# Set via environment for a session
export SUPP_PERF=balanced
supp why parse_config
```

## Technical details

### Thread pool (rayon)

supp uses [rayon](https://docs.rs/rayon) for data-parallel file processing during symbol indexing (`supp sym`, `supp why`) and context analysis (`supp <paths>`). By default, rayon spawns one thread per logical CPU core.

- **`full`**: all cores (rayon default - `num_threads = 0`)
- **`balanced`**: half of available cores, minimum 2
- **`lite`**: 2 threads

The global thread pool is configured once at startup. This directly controls CPU utilization - switching from `full` to `lite` on an 8-core machine drops peak CPU from ~800% to ~200%.

### PageRank iterations

Symbol search results (`supp sym`) are ranked by cross-file importance using an iterative PageRank algorithm (damping factor 0.85). Each iteration refines the ranking by propagating "importance" through the dependency graph - functions called from many files rank higher.

- **`full`**: 15 iterations - fully converged for most codebases
- **`balanced`**: 8 iterations - close to converged, noticeably faster on large symbol graphs
- **`lite`**: 5 iterations - sufficient for relative ordering, minor accuracy loss on deeply connected graphs

The cost is O(iterations x edges). On a codebase with 100k symbols and dense cross-references, reducing from 15 to 5 iterations saves ~67% of ranking CPU time.

### Call sites early exit

`supp why` walks the entire codebase to find where a symbol is called. Each file is read and scanned for the symbol name, and matching lines are parsed with tree-sitter to find the enclosing function.

Without early exit (`full` mode), supp walks every file even after reaching the result cap. This ensures deterministic results (files are walked in sorted order) but is wasteful on large codebases where 30 matches are found in the first few thousand files.

- **`full`**: walks all files, truncates to 30 results at the end
- **`balanced`/`lite`**: stops walking as soon as the cap is reached (30 or 15 results)

On a 50k-file codebase, early exit can skip reading tens of thousands of files if matches are found early.

### Used-by scan

Context analysis (`supp <paths>`) includes a "used by" section showing where the analyzed symbols are referenced across the project. This scan is O(N x M) where N is the number of analyzed files and M is the total project file count - for each analyzed file, supp walks the entire project looking for references.

- **`full`**: enabled when analyzing ≤20 files
- **`balanced`**: enabled when analyzing ≤10 files
- **`lite`**: disabled entirely

Disabling this scan removes the most expensive operation in context analysis on large codebases.

### File limits

Context analysis caps how many files and total bytes it processes:

- **`full`**: 20,000 files / 50 MB total content
- **`balanced`**: 50,000 files / 100 MB (higher limits since other knobs reduce work)
- **`lite`**: 10,000 files / 30 MB (strict limits to bound memory usage)

These limits apply to the files supp reads into memory for compression and analysis. Files beyond the limit produce an error suggesting you narrow your paths or use a filter regex.

## Resource cleanup

supp relies on Rust's ownership model (RAII) for resource cleanup. Tree-sitter parsers, git handles, and rayon worker threads are all freed automatically when they go out of scope. The symbol cache (`.git/supp/sym-cache`) persists between runs for incremental indexing - use `supp clean-cache` to remove it.

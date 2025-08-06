# 🎯 API Parity Enforcement for IronCalc

This document describes the automated API parity enforcement system for IronCalc bindings.

## 🚀 Quick Start

### Check API Parity
```bash
# Run complete parity check
python3 scripts/enforce_api_parity.py .

# Or use Makefile
make -f Makefile.api-parity check-full
```

### Install Git Hooks
```bash
# Install pre-commit hook
make -f Makefile.api-parity install-hooks

# Or manually
chmod +x scripts/pre-commit-api-parity.sh
ln -sf ../../scripts/pre-commit-api-parity.sh .git/hooks/pre-commit
```

## 📊 What Gets Checked

### ✅ Core Functions (REQUIRED)
These functions **must** exist in all bindings:

**Model (Raw API):**
- `set_user_input`, `get_cell_content`, `get_cell_type`
- `get_formatted_cell_value`, `evaluate`

**UserModel (User API):**
- `undo`, `redo`, `can_undo`, `can_redo`
- `new_sheet`, `delete_sheet`, `rename_sheet`
- `insert_rows`, `insert_columns`, `delete_rows`, `delete_columns`

### ⚠️ Utility Functions (WARNINGS)
These should exist but platform naming is allowed:
- `get_tokens` - formula tokenizer
- `column_name_from_number` / `column_name_from_number_js`

### 🔧 Defined Names (REQUIRED)
Complete CRUD operations:
- `new_defined_name`, `update_defined_name`
- `delete_defined_name` / `delete_definedname` (naming variants allowed)
- `get_defined_name_list`

## 🏗️ Architecture

### Files Structure
```
scripts/
├── enforce_api_parity.py      # Main enforcement script
└── pre-commit-api-parity.sh   # Git pre-commit hook

.github/workflows/
└── api-parity-check.yml       # CI/CD enforcement

api-parity-config.json          # Configuration file
Makefile.api-parity            # Make targets
```

### Enforcement Levels
- **ERROR**: Critical functions missing → CI fails
- **WARNING**: Utility functions missing → CI warns but passes
- **INFO**: Platform-specific differences → Informational only

## 🔧 Configuration

Edit `api-parity-config.json` to customize:

```json
{
  "rules": {
    "core_functions": {
      "severity": "error",
      "model_functions": ["set_user_input", "evaluate"],
      "usermodel_functions": ["undo", "redo"]
    },
    "platform_specific": {
      "nodejs_specific": ["from_xlsx", "new"],
      "python_specific": ["create", "load_from_xlsx"],
      "wasm_specific": ["new", "from_bytes"]
    }
  }
}
```

## 🚦 CI/CD Integration

### GitHub Actions
The workflow runs automatically on:
- Push to `main`/`develop` branches
- PRs targeting `main`/`develop`
- Changes to binding files (`bindings/**/*.rs`, `bindings/**/*.ts`)

### Local Development
Pre-commit hook prevents commits that break parity:
```bash
# This will run automatically before commits
git commit -m "Add new API function"
# 🔍 Pre-commit: Checking API parity...
# ✅ API parity check passed
# 🚀 Commit approved!
```

## 📊 Parity Matrix

The enforcement generates `API_PARITY_MATRIX.md`:

| Function | Node.js | WASM | Python | Status |
|----------|---------|------|--------|--------|
| `evaluate` | ✅ | ❌ | ✅ | ⚠️ PARTIAL |
| `undo` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |
| `get_tokens` | ✅ | ✅ | ✅ | 🎯 FULL PARITY |

Status meanings:
- 🎯 **FULL PARITY**: Function exists in all bindings
- ⚠️ **PARTIAL**: Missing in 1 binding
- ❌ **LIMITED**: Missing in 2+ bindings

## 🛠️ Available Commands

### Makefile Targets
```bash
# Full check (compilation + parity)
make -f Makefile.api-parity check-full

# Just parity checking
make -f Makefile.api-parity check-api-parity

# Generate matrix only
make -f Makefile.api-parity parity-matrix

# Install git hooks
make -f Makefile.api-parity install-hooks

# CI-ready check (fail on any issues)
make -f Makefile.api-parity ci-check

# Development check (warnings only)
make -f Makefile.api-parity dev-check
```

### Direct Script Usage
```bash
# Basic check
python3 scripts/enforce_api_parity.py .

# With custom repo path
python3 scripts/enforce_api_parity.py /path/to/ironcalc
```

## 🔍 How It Works

### 1. API Extraction
The script parses Rust source files to extract:
- `#[napi]` functions (Node.js)
- `#[wasm_bindgen]` functions (WASM)
- `#[pymethods]` functions (Python)

### 2. Parity Rules
Applies configurable rules:
- **Core functions**: Must exist everywhere
- **Utility functions**: Should exist (platform naming OK)
- **Platform-specific**: Allowed to differ

### 3. Reporting
Generates:
- Console output with violations
- `API_PARITY_MATRIX.md` with detailed breakdown
- Exit codes for CI/CD integration

## 🎯 Best Practices

### For Developers
1. **Install pre-commit hooks** for early detection
2. **Run `dev-check`** during development
3. **Check matrix** before major releases

### For Maintainers
1. **Require parity checks** in CI/CD
2. **Review matrix** in PR comments
3. **Update config** when adding new core functions

### For New Features
1. **Add to all bindings** simultaneously when possible
2. **Update config** if adding new core functions
3. **Document** platform-specific differences

## 🚨 Common Issues

### "Core function missing"
```bash
❌ Core function 'evaluate' missing in: wasm
```
**Solution**: Add the function to the missing binding(s)

### "Compilation failed"
```bash
❌ Python binding compilation failed
```
**Solution**: Fix compilation errors before checking parity

### "Function exists but not detected"
**Possible causes**:
- Missing `#[napi]`/`#[wasm_bindgen]`/`#[pymethods]` attribute
- Function in wrong impl block
- Typo in function name

## 📈 Benefits

### ✅ Automated Detection
- Catches parity issues immediately
- Prevents API drift between bindings
- Saves manual review time

### ✅ CI/CD Integration
- Blocks merging of parity-breaking changes
- Provides clear feedback in PRs
- Maintains consistent experience

### ✅ Documentation
- Auto-generated parity matrix
- Clear status of each function
- Historical tracking via git

## 🔮 Future Enhancements

Potential improvements:
- **Signature checking**: Verify parameter compatibility
- **Return type validation**: Ensure consistent return types
- **Documentation parity**: Check that functions are documented
- **Integration tests**: Automated cross-platform testing
- **Performance benchmarks**: Ensure consistent performance

---

**🎉 With this system, IronCalc maintains excellent API parity across all platforms automatically!**
#!/usr/bin/env python3
"""
API Parity Enforcement Tool for IronCalc Bindings

This script can be run in CI/CD to ensure all bindings maintain API parity.
It extracts APIs from all bindings and enforces parity rules.
"""

import os
import re
import sys
import json
import subprocess
from pathlib import Path
from typing import Dict, List, Set, Tuple
from dataclasses import dataclass

@dataclass
class APIFunction:
    name: str
    binding: str
    api_type: str  # 'model', 'usermodel', 'lib'
    signature: str = ""
    
@dataclass
class ParityRule:
    name: str
    description: str
    check_func: callable
    severity: str  # 'error', 'warning'

class APIParityEnforcer:
    def __init__(self, repo_root: str):
        self.repo_root = Path(repo_root)
        self.apis = {}
        self.violations = []
        self.rules = []
        self._setup_rules()
    
    def _setup_rules(self):
        """Define parity enforcement rules"""
        self.rules = [
            ParityRule(
                "core_model_parity",
                "Core Model functions must exist in all bindings",
                self._check_core_model_parity,
                "error"
            ),
            ParityRule(
                "core_usermodel_parity", 
                "Core UserModel functions must exist in all bindings",
                self._check_core_usermodel_parity,
                "error"
            ),
            ParityRule(
                "utility_function_parity",
                "Utility functions should exist in all bindings (with platform naming)",
                self._check_utility_parity,
                "warning"
            ),
            ParityRule(
                "defined_names_parity",
                "Defined names operations must be consistent",
                self._check_defined_names_parity,
                "error"
            ),
            ParityRule(
                "evaluation_parity",
                "Evaluation functions must exist in all bindings",
                self._check_evaluation_parity,
                "error"
            )
        ]
    
    def extract_nodejs_api(self) -> Dict[str, List[str]]:
        """Extract Node.js API from TypeScript definitions and Rust source"""
        apis = {"model": [], "usermodel": [], "lib": []}
        
        # Extract from Rust source files
        model_file = self.repo_root / "bindings/nodejs/src/model.rs"
        usermodel_file = self.repo_root / "bindings/nodejs/src/user_model.rs"
        lib_file = self.repo_root / "bindings/nodejs/src/lib.rs"
        
        if model_file.exists():
            apis["model"] = self._extract_rust_functions(model_file)
        
        if usermodel_file.exists():
            apis["usermodel"] = self._extract_rust_functions(usermodel_file)
            
        if lib_file.exists():
            apis["lib"] = self._extract_rust_lib_functions(lib_file)
        
        return apis
    
    def extract_wasm_api(self) -> Dict[str, List[str]]:
        """Extract WASM API from Rust source"""
        apis = {"model": [], "usermodel": [], "lib": []}
        
        wasm_file = self.repo_root / "bindings/wasm/src/lib.rs"
        if wasm_file.exists():
            content = wasm_file.read_text()
            
            # WASM has everything in one Model struct
            apis["usermodel"] = self._extract_wasm_functions(content)
            apis["lib"] = self._extract_wasm_lib_functions(content)
        
        return apis
    
    def extract_python_api(self) -> Dict[str, List[str]]:
        """Extract Python API from Rust source"""
        apis = {"model": [], "usermodel": [], "lib": []}
        
        python_file = self.repo_root / "bindings/python/src/lib.rs"
        if python_file.exists():
            content = python_file.read_text()
            
            # Extract PyModel functions
            apis["model"] = self._extract_python_model_functions(content)
            
            # Extract PyUserModel functions  
            apis["usermodel"] = self._extract_python_usermodel_functions(content)
            
            # Extract module-level functions
            apis["lib"] = self._extract_python_lib_functions(content)
        
        return apis
    
    def _extract_rust_functions(self, file_path: Path) -> List[str]:
        """Extract function names from Rust file with #[napi] attribute"""
        content = file_path.read_text()
        functions = []
        
        lines = content.split('\n')
        for i, line in enumerate(lines):
            if '#[napi' in line and i + 1 < len(lines):
                next_line = lines[i + 1]
                match = re.search(r'pub fn (\w+)', next_line)
                if match:
                    functions.append(match.group(1))
        
        return functions
    
    def _extract_rust_lib_functions(self, file_path: Path) -> List[str]:
        """Extract top-level functions from Node.js lib.rs"""
        content = file_path.read_text()
        functions = []
        
        # Look for #[napi] functions not in impl blocks
        lines = content.split('\n')
        in_impl = False
        brace_count = 0
        
        for line in lines:
            if 'impl ' in line and '{' in line:
                in_impl = True
                brace_count = line.count('{')
            elif in_impl:
                brace_count += line.count('{') - line.count('}')
                if brace_count <= 0:
                    in_impl = False
            elif '#[napi' in line:
                # Find the next function definition
                for next_line in lines[lines.index(line):]:
                    match = re.search(r'pub fn (\w+)', next_line)
                    if match:
                        functions.append(match.group(1))
                        break
        
        return functions
    
    def _extract_wasm_functions(self, content: str) -> List[str]:
        """Extract WASM functions with #[wasm_bindgen] attribute"""
        functions = []
        
        # Find impl Model block
        in_model_impl = False
        brace_count = 0
        
        lines = content.split('\n')
        for i, line in enumerate(lines):
            if 'impl Model {' in line:
                in_model_impl = True
                brace_count = 1
                continue
            elif in_model_impl:
                brace_count += line.count('{') - line.count('}')
                if brace_count <= 0:
                    break
                elif '#[wasm_bindgen' in line:
                    # Look for function in next few lines
                    for j in range(i+1, min(i+5, len(lines))):
                        match = re.search(r'pub fn (\w+)', lines[j])
                        if match:
                            functions.append(match.group(1))
                            break
        
        return functions
    
    def _extract_wasm_lib_functions(self, content: str) -> List[str]:
        """Extract WASM top-level functions"""
        functions = []
        
        # Look for #[wasm_bindgen] functions outside impl blocks
        lines = content.split('\n')
        in_impl = False
        brace_count = 0
        
        for i, line in enumerate(lines):
            if 'impl ' in line and '{' in line:
                in_impl = True
                brace_count = line.count('{')
            elif in_impl:
                brace_count += line.count('{') - line.count('}')
                if brace_count <= 0:
                    in_impl = False
            elif '#[wasm_bindgen' in line and not in_impl:
                # Look for function in next few lines
                for j in range(i+1, min(i+5, len(lines))):
                    match = re.search(r'pub fn (\w+)', lines[j])
                    if match:
                        functions.append(match.group(1))
                        break
        
        return functions
    
    def _extract_python_model_functions(self, content: str) -> List[str]:
        """Extract PyModel functions"""
        functions = []
        in_pymodel_impl = False
        brace_count = 0
        
        lines = content.split('\n')
        for i, line in enumerate(lines):
            if 'impl PyModel {' in line:
                in_pymodel_impl = True
                brace_count = 1
                continue
            elif in_pymodel_impl:
                brace_count += line.count('{') - line.count('}')
                if brace_count <= 0:
                    break
                elif 'pub fn' in line:
                    match = re.search(r'pub fn (\w+)', line)
                    if match:
                        functions.append(match.group(1))
        
        return functions
    
    def _extract_python_usermodel_functions(self, content: str) -> List[str]:
        """Extract PyUserModel functions"""
        functions = []
        in_pyusermodel_impl = False
        brace_count = 0
        
        lines = content.split('\n')
        for i, line in enumerate(lines):
            if 'impl PyUserModel {' in line:
                in_pyusermodel_impl = True
                brace_count = 1
                continue
            elif in_pyusermodel_impl:
                brace_count += line.count('{') - line.count('}')
                if brace_count <= 0:
                    break
                elif 'pub fn' in line:
                    match = re.search(r'pub fn (\w+)', line)
                    if match:
                        functions.append(match.group(1))
        
        return functions
    
    def _extract_python_lib_functions(self, content: str) -> List[str]:
        """Extract Python module-level functions"""
        functions = []
        
        # Look for #[pyfunction] decorated functions
        lines = content.split('\n')
        for i, line in enumerate(lines):
            if '#[pyfunction]' in line and i + 1 < len(lines):
                next_line = lines[i + 1]
                match = re.search(r'pub fn (\w+)', next_line)
                if match:
                    functions.append(match.group(1))
        
        return functions
    
    def extract_all_apis(self):
        """Extract APIs from all bindings"""
        print("🔍 Extracting APIs from all bindings...")
        
        self.apis["nodejs"] = self.extract_nodejs_api()
        self.apis["wasm"] = self.extract_wasm_api() 
        self.apis["python"] = self.extract_python_api()
        
        print(f"📊 API Extraction Results:")
        for binding, api_types in self.apis.items():
            for api_type, functions in api_types.items():
                print(f"   {binding}.{api_type}: {len(functions)} functions")
    
    def _check_core_model_parity(self) -> List[str]:
        """Check that core Model functions exist in all bindings"""
        violations = []
        
        # Define core Model functions that should exist everywhere
        core_functions = {
            'set_user_input', 'get_cell_content', 'get_cell_type', 
            'get_formatted_cell_value', 'evaluate'
        }
        
        for func in core_functions:
            missing_bindings = []
            for binding in ['nodejs', 'wasm', 'python']:
                # Check both model and usermodel APIs (WASM has everything in usermodel)
                model_funcs = self.apis.get(binding, {}).get('model', [])
                usermodel_funcs = self.apis.get(binding, {}).get('usermodel', [])
                
                if func not in model_funcs and func not in usermodel_funcs:
                    missing_bindings.append(binding)
            
            if missing_bindings:
                violations.append(f"Core function '{func}' missing in: {', '.join(missing_bindings)}")
        
        return violations
    
    def _check_core_usermodel_parity(self) -> List[str]:
        """Check that core UserModel functions exist in all bindings"""
        violations = []
        
        # Define core UserModel functions
        core_functions = {
            'undo', 'redo', 'can_undo', 'can_redo',
            'new_sheet', 'delete_sheet', 'rename_sheet',
            'insert_rows', 'insert_columns', 'delete_rows', 'delete_columns'
        }
        
        for func in core_functions:
            missing_bindings = []
            for binding in ['nodejs', 'wasm', 'python']:
                usermodel_funcs = self.apis.get(binding, {}).get('usermodel', [])
                
                if func not in usermodel_funcs:
                    missing_bindings.append(binding)
            
            if missing_bindings:
                violations.append(f"Core UserModel function '{func}' missing in: {', '.join(missing_bindings)}")
        
        return violations
    
    def _check_utility_parity(self) -> List[str]:
        """Check utility function parity (allowing for platform naming)"""
        violations = []
        
        # Check for get_tokens utility
        nodejs_has_tokens = 'get_tokens' in self.apis.get('nodejs', {}).get('lib', [])
        wasm_has_tokens = 'get_tokens' in self.apis.get('wasm', {}).get('lib', [])
        python_has_tokens = 'get_tokens' in self.apis.get('python', {}).get('lib', [])
        
        if not all([nodejs_has_tokens, wasm_has_tokens, python_has_tokens]):
            missing = []
            if not nodejs_has_tokens: missing.append('nodejs')
            if not wasm_has_tokens: missing.append('wasm') 
            if not python_has_tokens: missing.append('python')
            violations.append(f"get_tokens utility missing in: {', '.join(missing)}")
        
        # Check for column name utility (allowing platform naming)
        nodejs_has_col = any(name in self.apis.get('nodejs', {}).get('lib', []) 
                           for name in ['column_name_from_number_js', 'column_name_from_number'])
        wasm_has_col = 'column_name_from_number' in self.apis.get('wasm', {}).get('lib', [])
        python_has_col = 'column_name_from_number' in self.apis.get('python', {}).get('lib', [])
        
        if not all([nodejs_has_col, wasm_has_col, python_has_col]):
            missing = []
            if not nodejs_has_col: missing.append('nodejs')
            if not wasm_has_col: missing.append('wasm')
            if not python_has_col: missing.append('python')
            violations.append(f"column_name_from_number utility missing in: {', '.join(missing)}")
        
        return violations
    
    def _check_defined_names_parity(self) -> List[str]:
        """Check defined names operations parity"""
        violations = []
        
        required_funcs = {'new_defined_name', 'update_defined_name', 'get_defined_name_list'}
        
        for func in required_funcs:
            missing_bindings = []
            for binding in ['nodejs', 'wasm', 'python']:
                # Check both model and usermodel APIs
                model_funcs = self.apis.get(binding, {}).get('model', [])
                usermodel_funcs = self.apis.get(binding, {}).get('usermodel', [])
                
                # Allow for slight naming variations
                variations = [func, func.replace('_', '')]
                has_func = any(var in model_funcs or var in usermodel_funcs for var in variations)
                
                if not has_func:
                    missing_bindings.append(binding)
            
            if missing_bindings:
                violations.append(f"Defined names function '{func}' missing in: {', '.join(missing_bindings)}")
        
        return violations
    
    def _check_evaluation_parity(self) -> List[str]:
        """Check evaluation function parity"""
        violations = []
        
        for binding in ['nodejs', 'wasm', 'python']:
            model_funcs = self.apis.get(binding, {}).get('model', [])
            usermodel_funcs = self.apis.get(binding, {}).get('usermodel', [])
            
            has_evaluate = 'evaluate' in model_funcs or 'evaluate' in usermodel_funcs
            
            if not has_evaluate:
                violations.append(f"'evaluate' function missing in {binding} binding")
        
        return violations
    
    def check_parity(self) -> bool:
        """Run all parity checks"""
        print("\n🔍 Running API Parity Checks...")
        
        all_passed = True
        
        for rule in self.rules:
            print(f"\n📋 Checking: {rule.description}")
            violations = rule.check_func()
            
            if violations:
                symbol = "❌" if rule.severity == "error" else "⚠️"
                print(f"{symbol} {len(violations)} violation(s) found:")
                for violation in violations:
                    print(f"   • {violation}")
                
                if rule.severity == "error":
                    all_passed = False
            else:
                print("✅ PASSED")
        
        return all_passed
    
    def generate_parity_matrix(self) -> str:
        """Generate a parity matrix showing function coverage"""
        matrix = []
        matrix.append("# 📊 API Parity Matrix\n")
        
        # Get all unique functions
        all_functions = set()
        for binding_apis in self.apis.values():
            for api_funcs in binding_apis.values():
                all_functions.update(api_funcs)
        
        all_functions = sorted(all_functions)
        
        # Create matrix table
        matrix.append("| Function | Node.js | WASM | Python | Status |")
        matrix.append("|----------|---------|------|--------|--------|")
        
        for func in all_functions:
            nodejs_model = func in self.apis.get('nodejs', {}).get('model', [])
            nodejs_user = func in self.apis.get('nodejs', {}).get('usermodel', [])
            nodejs_lib = func in self.apis.get('nodejs', {}).get('lib', [])
            nodejs_has = nodejs_model or nodejs_user or nodejs_lib
            
            wasm_user = func in self.apis.get('wasm', {}).get('usermodel', [])
            wasm_lib = func in self.apis.get('wasm', {}).get('lib', [])
            wasm_has = wasm_user or wasm_lib
            
            python_model = func in self.apis.get('python', {}).get('model', [])
            python_user = func in self.apis.get('python', {}).get('usermodel', [])
            python_lib = func in self.apis.get('python', {}).get('lib', [])
            python_has = python_model or python_user or python_lib
            
            nodejs_mark = "✅" if nodejs_has else "❌"
            wasm_mark = "✅" if wasm_has else "❌"
            python_mark = "✅" if python_has else "❌"
            
            parity_count = sum([nodejs_has, wasm_has, python_has])
            if parity_count == 3:
                status = "🎯 FULL PARITY"
            elif parity_count == 2:
                status = "⚠️ PARTIAL"
            else:
                status = "❌ LIMITED"
            
            matrix.append(f"| `{func}` | {nodejs_mark} | {wasm_mark} | {python_mark} | {status} |")
        
        return "\n".join(matrix)
    
    def run_enforcement(self, warning_mode: bool = False) -> int:
        """Run complete API parity enforcement"""
        print("🚀 IronCalc API Parity Enforcement")
        print("=" * 50)
        
        try:
            # Extract APIs
            self.extract_all_apis()
            
            # Check parity
            parity_passed = self.check_parity()
            
            # Generate matrix report
            matrix = self.generate_parity_matrix()
            with open(self.repo_root / "API_PARITY_MATRIX.md", "w") as f:
                f.write(matrix)
            print(f"\n📊 Parity matrix saved to API_PARITY_MATRIX.md")
            
            # Final verdict
            print("\n" + "=" * 50)
            if parity_passed:
                print("🎉 API PARITY ENFORCEMENT: PASSED")
                print("✅ All critical parity checks passed!")
                return 0
            else:
                if warning_mode:
                    print("⚠️ API PARITY ENFORCEMENT: WARNINGS DETECTED")
                    print("💡 Parity violations found but proceeding (warning mode)")
                    return 0
                else:
                    print("❌ API PARITY ENFORCEMENT: FAILED")
                    print("⚠️ Critical parity violations detected!")
                    return 1
                
        except Exception as e:
            print(f"💥 Error during enforcement: {e}")
            return 2

def main():
    import argparse
    parser = argparse.ArgumentParser(description='Enforce API parity across IronCalc bindings')
    parser.add_argument('repo_path', nargs='?', default=os.getcwd(), 
                       help='Path to IronCalc repository (default: current directory)')
    parser.add_argument('--warning-mode', action='store_true',
                       help='Run in warning mode (exit 0 even with parity issues)')
    
    args = parser.parse_args()
    
    enforcer = APIParityEnforcer(args.repo_path)
    return enforcer.run_enforcement(warning_mode=args.warning_mode)

if __name__ == "__main__":
    sys.exit(main())
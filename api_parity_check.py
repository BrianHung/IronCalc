#!/usr/bin/env python3
"""
Comprehensive API parity checker across Node.js, WASM, and Python bindings.
This script extracts and compares the exact API surface of all three bindings.
"""

import re
import json
from pathlib import Path

def extract_nodejs_model_api():
    """Extract Node.js Model API functions"""
    model_file = Path("/Users/brianhung/IronCalc/bindings/nodejs/src/model.rs")
    content = model_file.read_text()
    
    # Extract function names from #[napi] annotated functions
    functions = []
    lines = content.split('\n')
    for i, line in enumerate(lines):
        if '#[napi]' in line or '#[napi(' in line:
            # Look for the next pub fn
            for j in range(i+1, min(i+5, len(lines))):
                if 'pub fn' in lines[j]:
                    match = re.search(r'pub fn (\w+)', lines[j])
                    if match:
                        functions.append(match.group(1))
                    break
    
    # Also check for factory and constructor methods
    factory_constructor = re.findall(r'#\[napi\((?:factory|constructor)\)\]\s*pub fn (\w+)', content)
    functions.extend(factory_constructor)
    
    return sorted(set(functions))

def extract_nodejs_usermodel_api():
    """Extract Node.js UserModel API functions"""
    usermodel_file = Path("/Users/brianhung/IronCalc/bindings/nodejs/src/user_model.rs")
    content = usermodel_file.read_text()
    
    functions = []
    lines = content.split('\n')
    for i, line in enumerate(lines):
        if '#[napi]' in line or '#[napi(' in line:
            # Look for the next pub fn
            for j in range(i+1, min(i+5, len(lines))):
                if 'pub fn' in lines[j]:
                    match = re.search(r'pub fn (\w+)', lines[j])
                    if match:
                        functions.append(match.group(1))
                    break
    
    factory_constructor = re.findall(r'#\[napi\((?:factory|constructor)\)\]\s*pub fn (\w+)', content)
    functions.extend(factory_constructor)
    
    return sorted(set(functions))

def extract_nodejs_lib_api():
    """Extract Node.js lib-level functions"""
    lib_file = Path("/Users/brianhung/IronCalc/bindings/nodejs/src/lib.rs")
    content = lib_file.read_text()
    
    functions = []
    lines = content.split('\n')
    for i, line in enumerate(lines):
        if '#[napi(' in line and 'js_name' in line:
            # Look for the next pub fn
            for j in range(i+1, min(i+5, len(lines))):
                if 'pub fn' in lines[j]:
                    match = re.search(r'pub fn (\w+)', lines[j])
                    if match:
                        functions.append(match.group(1))
                    break
    
    return sorted(set(functions))

def extract_wasm_api():
    """Extract WASM API functions"""
    wasm_file = Path("/Users/brianhung/IronCalc/bindings/wasm/src/lib.rs")
    content = wasm_file.read_text()
    
    functions = []
    # Extract all #[wasm_bindgen] annotated functions
    wasm_functions = re.findall(r'#\[wasm_bindgen[^\]]*\]\s*pub fn (\w+)', content)
    functions.extend(wasm_functions)
    
    # Also check for constructor and js_name variants
    constructor_functions = re.findall(r'#\[wasm_bindgen\(constructor\)\]\s*pub fn (\w+)', content)
    functions.extend(constructor_functions)
    
    return sorted(set(functions))

def extract_python_model_api():
    """Extract Python PyModel API functions"""
    python_file = Path("/Users/brianhung/IronCalc/bindings/python/src/lib.rs")
    content = python_file.read_text()
    
    # Find PyModel impl block and extract methods
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
            # Count braces to know when impl block ends
            brace_count += line.count('{') - line.count('}')
            if brace_count <= 0:
                break
            elif 'pub fn' in line:
                match = re.search(r'pub fn (\w+)', line)
                if match:
                    functions.append(match.group(1))
    
    return sorted(set(functions))

def extract_python_usermodel_api():
    """Extract Python PyUserModel API functions"""
    python_file = Path("/Users/brianhung/IronCalc/bindings/python/src/lib.rs")
    content = python_file.read_text()
    
    # Find PyUserModel impl block and extract methods
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
            # Count braces to know when impl block ends
            brace_count += line.count('{') - line.count('}')
            if brace_count <= 0:
                break
            elif 'pub fn' in line:
                match = re.search(r'pub fn (\w+)', line)
                if match:
                    functions.append(match.group(1))
    
    return sorted(set(functions))

def extract_python_lib_api():
    """Extract Python module-level functions"""
    python_file = Path("/Users/brianhung/IronCalc/bindings/python/src/lib.rs")
    content = python_file.read_text()
    
    functions = []
    # Find #[pyfunction] annotated functions
    pyfunction_matches = re.findall(r'#\[pyfunction\]\s*pub fn (\w+)', content)
    functions.extend(pyfunction_matches)
    
    return sorted(set(functions))

def normalize_function_name(name):
    """Normalize function names for comparison (camelCase to snake_case)"""
    # Convert camelCase to snake_case
    normalized = re.sub('(.)([A-Z][a-z]+)', r'\1_\2', name)
    normalized = re.sub('([a-z0-9])([A-Z])', r'\1_\2', normalized).lower()
    return normalized

def compare_apis():
    """Compare all APIs and report differences"""
    print("🔍 Extracting API surfaces from all bindings...\n")
    
    # Extract all APIs
    node_model = extract_nodejs_model_api()
    node_usermodel = extract_nodejs_usermodel_api()
    node_lib = extract_nodejs_lib_api()
    
    wasm = extract_wasm_api()
    
    python_model = extract_python_model_api()
    python_usermodel = extract_python_usermodel_api()
    python_lib = extract_python_lib_api()
    
    print(f"📊 **API Function Counts:**")
    print(f"Node.js Model: {len(node_model)} functions")
    print(f"Node.js UserModel: {len(node_usermodel)} functions")
    print(f"Node.js Lib: {len(node_lib)} functions")
    print(f"WASM: {len(wasm)} functions")
    print(f"Python PyModel: {len(python_model)} functions")
    print(f"Python PyUserModel: {len(python_usermodel)} functions")
    print(f"Python Lib: {len(python_lib)} functions")
    print()
    
    # Normalize function names for comparison
    def normalize_set(func_set):
        return {normalize_function_name(f) for f in func_set}
    
    node_model_norm = normalize_set(node_model)
    node_usermodel_norm = normalize_set(node_usermodel)
    node_lib_norm = normalize_set(node_lib)
    
    wasm_norm = normalize_set(wasm)
    
    python_model_norm = normalize_set(python_model)
    python_usermodel_norm = normalize_set(python_usermodel)
    python_lib_norm = normalize_set(python_lib)
    
    # Create comprehensive sets for comparison
    node_total = node_model_norm | node_usermodel_norm | node_lib_norm
    wasm_total = wasm_norm
    python_total = python_model_norm | python_usermodel_norm | python_lib_norm
    
    print("🎯 **Comprehensive API Parity Analysis:**")
    print()
    
    # Check Node vs WASM
    node_wasm_missing_in_wasm = node_total - wasm_total
    node_wasm_missing_in_node = wasm_total - node_total
    
    print("**Node.js ↔ WASM Comparison:**")
    if node_wasm_missing_in_wasm:
        print(f"❌ Missing in WASM: {sorted(node_wasm_missing_in_wasm)}")
    else:
        print("✅ WASM has all Node.js functions")
        
    if node_wasm_missing_in_node:
        print(f"❌ Missing in Node.js: {sorted(node_wasm_missing_in_node)}")
    else:
        print("✅ Node.js has all WASM functions")
    print()
    
    # Check Node vs Python
    node_python_missing_in_python = node_total - python_total
    node_python_missing_in_node = python_total - node_total
    
    print("**Node.js ↔ Python Comparison:**")
    if node_python_missing_in_python:
        print(f"❌ Missing in Python: {sorted(node_python_missing_in_python)}")
    else:
        print("✅ Python has all Node.js functions")
        
    if node_python_missing_in_node:
        print(f"❌ Missing in Node.js: {sorted(node_python_missing_in_node)}")
    else:
        print("✅ Node.js has all Python functions")
    print()
    
    # Check WASM vs Python
    wasm_python_missing_in_python = wasm_total - python_total
    wasm_python_missing_in_wasm = python_total - wasm_total
    
    print("**WASM ↔ Python Comparison:**")
    if wasm_python_missing_in_python:
        print(f"❌ Missing in Python: {sorted(wasm_python_missing_in_python)}")
    else:
        print("✅ Python has all WASM functions")
        
    if wasm_python_missing_in_wasm:
        print(f"❌ Missing in WASM: {sorted(wasm_python_missing_in_wasm)}")
    else:
        print("✅ WASM has all Python functions")
    print()
    
    # Overall parity check
    all_same = (len(node_wasm_missing_in_wasm) == 0 and len(node_wasm_missing_in_node) == 0 and
                len(node_python_missing_in_python) == 0 and len(node_python_missing_in_node) == 0 and
                len(wasm_python_missing_in_python) == 0 and len(wasm_python_missing_in_wasm) == 0)
    
    if all_same:
        print("🎉 **PERFECT API PARITY ACHIEVED!**")
        print("All three bindings (Node.js, WASM, Python) have identical API surfaces!")
    else:
        print("⚠️  **API PARITY GAPS DETECTED**")
        print("Some functions are missing between bindings.")
    
    print()
    
    # Detailed breakdown
    print("📋 **Detailed Function Lists:**")
    print()
    print(f"**Node.js Model ({len(node_model)}):** {node_model}")
    print(f"**Node.js UserModel ({len(node_usermodel)}):** {node_usermodel}")
    print(f"**Node.js Lib ({len(node_lib)}):** {node_lib}")
    print()
    print(f"**WASM ({len(wasm)}):** {wasm}")
    print()
    print(f"**Python PyModel ({len(python_model)}):** {python_model}")
    print(f"**Python PyUserModel ({len(python_usermodel)}):** {python_usermodel}")
    print(f"**Python Lib ({len(python_lib)}):** {python_lib}")
    
    return all_same

if __name__ == "__main__":
    success = compare_apis()
    exit(0 if success else 1)
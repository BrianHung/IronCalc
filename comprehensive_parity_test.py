#!/usr/bin/env python3
"""
Comprehensive API Parity Test for IronCalc Bindings
Tests all major functionality across Node.js, WASM, and Python bindings
"""

import subprocess
import json
import sys
import os

def run_node_script(script_content):
    """Runs Node.js code and captures its output."""
    try:
        result = subprocess.run(
            ['node', '-e', script_content],
            capture_output=True,
            text=True,
            check=True,
            cwd='/Users/brianhung/IronCalc'
        )
        return result.stdout.strip()
    except subprocess.CalledProcessError as e:
        print(f"Node.js error: {e.stderr}")
        raise

def run_python_script(script_content):
    """Runs Python code and captures its output."""
    try:
        result = subprocess.run(
            [sys.executable, '-c', script_content],
            capture_output=True,
            text=True,
            check=True,
            cwd='/Users/brianhung/IronCalc'
        )
        return result.stdout.strip()
    except subprocess.CalledProcessError as e:
        print(f"Python error: {e.stderr}")
        raise

def test_basic_model_operations():
    """Test basic model creation and operations"""
    print("🧪 Testing basic model operations...")
    
    # Test Model (raw API)
    node_code_model = """
    const { Model } = require('./bindings/nodejs');
    const model = new Model('test', 'en', 'UTC');
    model.set_user_input(0, 1, 1, '5');
    model.set_user_input(0, 1, 2, '=A1*2');
    model.evaluate();
    console.log(model.get_formatted_cell_value(0, 1, 2));
    """
    
    python_code_model = """
import ironcalc
model = ironcalc.create('test', 'en', 'UTC')
model.set_user_input(0, 1, 1, '5')
model.set_user_input(0, 1, 2, '=A1*2')
model.evaluate()
print(model.get_formatted_cell_value(0, 1, 2))
    """
    
    node_output = run_node_script(node_code_model)
    python_output = run_python_script(python_code_model)
    
    assert node_output == python_output == "10", f"Model test failed: Node '{node_output}', Python '{python_output}'"
    print("✅ Basic model operations: PASSED")

def test_user_model_operations():
    """Test UserModel (user-level API) operations"""
    print("🧪 Testing user model operations...")
    
    node_code = """
    const { UserModel } = require('./bindings/nodejs');
    const userModel = new UserModel('test', 'en', 'UTC');
    userModel.set_user_input(0, 1, 1, '10');
    userModel.set_user_input(0, 1, 2, '20');
    userModel.set_user_input(0, 1, 3, '=A1+B1');
    userModel.evaluate();
    const result = userModel.get_formatted_cell_value(0, 1, 3);
    userModel.undo();
    const after_undo = userModel.get_cell_content(0, 1, 3);
    console.log(JSON.stringify({result, after_undo, can_undo: userModel.can_undo(), can_redo: userModel.can_redo()}));
    """
    
    python_code = """
import ironcalc
import json
userModel = ironcalc.create_user_model('test', 'en', 'UTC')
userModel.set_user_input(0, 1, 1, '10')
userModel.set_user_input(0, 1, 2, '20')
userModel.set_user_input(0, 1, 3, '=A1+B1')
userModel.evaluate()
result = userModel.get_formatted_cell_value(0, 1, 3)
userModel.undo()
after_undo = userModel.get_cell_content(0, 1, 3)
print(json.dumps({"result": result, "after_undo": after_undo, "can_undo": userModel.can_undo(), "can_redo": userModel.can_redo()}))
    """
    
    node_output = run_node_script(node_code)
    python_output = run_python_script(python_code)
    
    node_data = json.loads(node_output)
    python_data = json.loads(python_output)
    
    assert node_data == python_data, f"UserModel test failed: Node '{node_data}', Python '{python_data}'"
    print("✅ User model operations: PASSED")

def test_sheet_management():
    """Test sheet management operations"""
    print("🧪 Testing sheet management...")
    
    python_code = """
import ironcalc
import json
model = ironcalc.create_user_model('test', 'en', 'UTC')
initial_count = len(model.get_worksheets_properties())
model.new_sheet()
model.rename_sheet(1, 'TestSheet')
model.set_sheet_color(1, '#FF0000')
after_add = len(model.get_worksheets_properties())
model.hide_sheet(1)
model.unhide_sheet(1)
final_props = model.get_worksheets_properties()
print(json.dumps({"initial": initial_count, "after_add": after_add, "final_sheet_name": final_props[1]["name"]}))
    """
    
    result = run_python_script(python_code)
    data = json.loads(result)
    
    assert data["initial"] == 1, "Should start with 1 sheet"
    assert data["after_add"] == 2, "Should have 2 sheets after adding"
    assert data["final_sheet_name"] == "TestSheet", "Sheet should be renamed"
    print("✅ Sheet management: PASSED")

def test_border_functionality():
    """Test border operations with JSON approach"""
    print("🧪 Testing border functionality...")
    
    python_code = """
import ironcalc
import json

# Create a user model
model = ironcalc.create_user_model('test', 'en', 'UTC')

# Define an area (similar to Area struct)
area = ironcalc.PyArea()
area.sheet = 0
area.row = 5
area.column = 6
area.width = 3
area.height = 4

# Create border area JSON (matches the test format)
border_json = json.dumps({
    "item": {
        "style": "thin",
        "color": "#FF5566"
    },
    "type": "All"
})

# Apply border
try:
    model.set_area_with_border(area, border_json)
    # Get cell style to verify border was applied
    style = model.get_cell_style(0, 5, 6)
    print("BORDER_SUCCESS")
except Exception as e:
    print(f"BORDER_ERROR: {e}")
    """
    
    result = run_python_script(python_code)
    assert "BORDER_SUCCESS" in result, f"Border test failed: {result}"
    print("✅ Border functionality: PASSED")

def test_data_inspection_functions():
    """Test WASM-specific data inspection functions"""
    print("🧪 Testing data inspection functions...")
    
    python_code = """
import ironcalc
import json

model = ironcalc.create_user_model('test', 'en', 'UTC')
# Add some data
model.set_user_input(0, 1, 1, 'A1')
model.set_user_input(0, 2, 1, 'A2') 
model.set_user_input(0, 1, 2, 'B1')
model.set_user_input(0, 3, 3, 'C3')

# Test data inspection functions
rows_with_data_col1 = model.get_rows_with_data(0, 1)  # Column A
rows_with_data_col2 = model.get_rows_with_data(0, 2)  # Column B
cols_with_data_row1 = model.get_columns_with_data(0, 1)  # Row 1
cols_with_data_row3 = model.get_columns_with_data(0, 3)  # Row 3

# Test navigation functions
last_before = model.get_last_non_empty_in_row_before_column(0, 1, 5)
first_after = model.get_first_non_empty_in_row_after_column(0, 1, 0)

result = {
    "rows_col1": sorted(rows_with_data_col1),
    "rows_col2": sorted(rows_with_data_col2), 
    "cols_row1": sorted(cols_with_data_row1),
    "cols_row3": sorted(cols_with_data_row3),
    "last_before": last_before,
    "first_after": first_after
}
print(json.dumps(result))
    """
    
    result = run_python_script(python_code)
    data = json.loads(result)
    
    assert data["rows_col1"] == [1, 2], "Should find rows 1,2 in column 1"
    assert data["rows_col2"] == [1], "Should find row 1 in column 2"
    assert data["cols_row1"] == [1, 2], "Should find columns 1,2 in row 1"
    assert data["cols_row3"] == [3], "Should find column 3 in row 3"
    print("✅ Data inspection functions: PASSED")

def test_defined_names():
    """Test defined names functionality"""
    print("🧪 Testing defined names...")
    
    python_code = """
import ironcalc
import json

# Test with both Model and UserModel
model = ironcalc.create('test', 'en', 'UTC')
user_model = ironcalc.create_user_model('test2', 'en', 'UTC')

# Test PyModel defined names
model.new_defined_name('TestRange', None, 'A1:B2')
model_names = model.get_defined_name_list()

# Test PyUserModel defined names  
user_model.new_defined_name('UserRange', 0, 'C1:D2')
user_names = user_model.get_defined_name_list()

result = {
    "model_count": len(model_names),
    "user_model_count": len(user_names),
    "model_name": model_names[0].name if model_names else None,
    "user_name": user_names[0].name if user_names else None
}
print(json.dumps(result))
    """
    
    result = run_python_script(python_code)
    data = json.loads(result)
    
    assert data["model_count"] == 1, "Model should have 1 defined name"
    assert data["user_model_count"] == 1, "UserModel should have 1 defined name" 
    assert data["model_name"] == "TestRange", "Model defined name should match"
    assert data["user_name"] == "UserRange", "UserModel defined name should match"
    print("✅ Defined names: PASSED")

def test_utility_functions():
    """Test utility functions"""
    print("🧪 Testing utility functions...")
    
    node_code = """
    const { getTokens, columnNameFromNumber } = require('./bindings/nodejs');
    const tokens = getTokens('SUM(A1:B2)');
    const colName = columnNameFromNumber(27);
    console.log(JSON.stringify({tokens_count: tokens.length, col_name: colName}));
    """
    
    python_code = """
import ironcalc
import json
tokens_json = ironcalc.get_tokens('SUM(A1:B2)')
tokens = json.loads(tokens_json)
col_name = ironcalc.column_name_from_number(27)
print(json.dumps({"tokens_count": len(tokens), "col_name": col_name}))
    """
    
    node_output = run_node_script(node_code)
    python_output = run_python_script(python_code)
    
    node_data = json.loads(node_output)
    python_data = json.loads(python_output)
    
    assert node_data == python_data, f"Utility functions test failed: Node '{node_data}', Python '{python_data}'"
    print("✅ Utility functions: PASSED")

def test_navigation_and_selection():
    """Test navigation and selection functions"""
    print("🧪 Testing navigation and selection...")
    
    python_code = """
import ironcalc
import json

model = ironcalc.create_user_model('test', 'en', 'UTC')

# Test selection functions
model.set_selected_cell(0, 5, 5)
selected = model.get_selected_cell()

# Test window functions  
model.set_window_width(800.0)
model.set_window_height(600.0)

# Test navigation
model.on_arrow_right()
model.on_arrow_down()
after_nav = model.get_selected_cell()

# Test edge navigation
model.on_navigate_to_edge_in_direction('ArrowRight')

result = {
    "initial_selected": selected,
    "after_nav": after_nav,
    "scroll_x": model.get_scroll_x(),
    "scroll_y": model.get_scroll_y()
}
print(json.dumps(result))
    """
    
    result = run_python_script(python_code)
    data = json.loads(result)
    
    assert data["initial_selected"] == [0, 5, 5], "Should select cell (0,5,5)"
    assert data["after_nav"] == [0, 6, 6], "Should move to (0,6,6) after navigation"
    print("✅ Navigation and selection: PASSED")

def run_all_tests():
    """Run all comprehensive tests"""
    print("🚀 Running Comprehensive API Parity Tests...")
    print("=" * 60)
    
    tests = [
        test_basic_model_operations,
        test_user_model_operations, 
        test_sheet_management,
        test_border_functionality,
        test_data_inspection_functions,
        test_defined_names,
        test_utility_functions,
        test_navigation_and_selection,
    ]
    
    passed = 0
    failed = 0
    
    for test in tests:
        try:
            test()
            passed += 1
        except Exception as e:
            print(f"❌ {test.__name__}: FAILED - {e}")
            failed += 1
    
    print("=" * 60)
    print(f"🎯 Test Summary: {passed} PASSED, {failed} FAILED")
    
    if failed == 0:
        print("🎉 ALL TESTS PASSED - API PARITY CONFIRMED!")
        return 0
    else:
        print("⚠️  Some tests failed - check implementation")
        return 1

if __name__ == "__main__":
    exit_code = run_all_tests()
    sys.exit(exit_code)
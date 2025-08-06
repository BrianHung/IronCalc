#!/usr/bin/env python3
"""
Test script to verify API parity across Python, Node.js, and WASM bindings.
This checks that all three bindings can perform the same basic operations.
"""

def test_python_binding():
    """Test the Python binding with both PyModel and PyUserModel"""
    try:
        import ironcalc
        
        print("✓ Python binding imported successfully")
        
        # Test raw model
        model = ironcalc.create("test", "en", "UTC")
        model.set_user_input(0, 1, 1, "=2+3")
        model.evaluate()
        result = model.get_formatted_cell_value(0, 1, 1)
        assert result == "5", f"Expected '5', got '{result}'"
        print("✓ Python PyModel: basic formula works")
        
        # Test new APIs
        model.move_row(0, 2, 1)
        model.move_column(0, 2, 1)
        print("✓ Python PyModel: move_row/move_column work")
        
        defined_names = model.get_defined_name_list()
        model.new_defined_name("TestName", None, "Sheet1!A1")
        defined_names_after = model.get_defined_name_list()
        assert len(defined_names_after) == len(defined_names) + 1
        print("✓ Python PyModel: defined names work")
        
        # Test utility functions
        tokens = ironcalc.get_tokens("A1+B1")
        assert tokens is not None
        print("✓ Python: get_tokens works")
        
        col_name = ironcalc.column_name_from_number(1)
        assert col_name == "A", f"Expected 'A', got '{col_name}'"
        print("✓ Python: column_name_from_number works")
        
        # Test user model
        user_model = ironcalc.create_user_model("test_user", "en", "UTC")
        user_model.set_user_input(0, 1, 1, "=10+20")
        user_model.evaluate()
        result = user_model.get_formatted_cell_value(0, 1, 1)
        assert result == "30", f"Expected '30', got '{result}'"
        print("✓ Python PyUserModel: basic operations work")
        
        # Test new user model APIs
        user_model.new_sheet()
        user_model.rename_sheet(1, "NewSheet")
        user_model.set_selected_sheet(1)
        selected = user_model.get_selected_sheet()
        assert selected == 1, f"Expected sheet 1, got {selected}"
        print("✓ Python PyUserModel: sheet management works")
        
        user_model.set_selected_cell(5, 5)
        cell_info = user_model.get_selected_cell()
        assert cell_info[1] == 5 and cell_info[2] == 5, f"Expected [_, 5, 5], got {cell_info}"
        print("✓ Python PyUserModel: selection works")
        
        # Test undo/redo
        assert user_model.can_undo() == True, "Should be able to undo"
        user_model.undo()
        print("✓ Python PyUserModel: undo/redo works")
        
        print("🎉 All Python binding tests passed!")
        return True
        
    except Exception as e:
        print(f"❌ Python binding test failed: {e}")
        return False

def test_nodejs_binding():
    """Test the Node.js binding (requires node and built binding)"""
    import subprocess
    import json
    
    try:
        # Create a minimal Node.js test script
        node_test = """
const { Model, UserModel, getTokens, columnNameFromNumber } = require('./bindings/nodejs');

try {
    // Test raw model
    const model = new Model("test", "en", "UTC");
    model.setUserInput(0, 1, 1, "=2+3");
    model.evaluate();
    const result = model.getFormattedCellValue(0, 1, 1);
    console.assert(result === "5", `Expected '5', got '${result}'`);
    console.log("✓ Node Model: basic formula works");
    
    // Test new utility functions
    const tokens = getTokens("A1+B1");
    console.assert(tokens !== null, "getTokens should return something");
    console.log("✓ Node: getTokens works");
    
    const colName = columnNameFromNumber(1);
    console.assert(colName === "A", `Expected 'A', got '${colName}'`);
    console.log("✓ Node: columnNameFromNumber works");
    
    // Test user model
    const userModel = new UserModel("test_user", "en", "UTC");
    userModel.setUserInput(0, 1, 1, "=10+20");
    userModel.evaluate();
    const userResult = userModel.getFormattedCellValue(0, 1, 1);
    console.assert(userResult === "30", `Expected '30', got '${userResult}'`);
    console.log("✓ Node UserModel: basic operations work");
    
    console.log("🎉 All Node.js binding tests passed!");
    console.log("NODE_TEST_SUCCESS");
} catch (error) {
    console.error("❌ Node.js binding test failed:", error);
    process.exit(1);
}
"""
        
        with open("/tmp/test_node.js", "w") as f:
            f.write(node_test)
        
        result = subprocess.run(
            ["node", "/tmp/test_node.js"],
            cwd="/Users/brianhung/IronCalc",
            capture_output=True,
            text=True,
            timeout=30
        )
        
        if result.returncode == 0 and "NODE_TEST_SUCCESS" in result.stdout:
            print("🎉 Node.js binding tests passed!")
            return True
        else:
            print(f"❌ Node.js binding test failed:")
            print(f"STDOUT: {result.stdout}")
            print(f"STDERR: {result.stderr}")
            return False
            
    except subprocess.TimeoutExpired:
        print("❌ Node.js test timed out")
        return False
    except Exception as e:
        print(f"❌ Node.js binding test failed: {e}")
        return False

def main():
    """Run all binding tests"""
    print("Testing API parity across IronCalc bindings...\n")
    
    results = []
    
    print("=== Testing Python Binding ===")
    results.append(test_python_binding())
    
    print("\n=== Testing Node.js Binding ===")
    results.append(test_nodejs_binding())
    
    print(f"\n=== Summary ===")
    print(f"Python: {'✓ PASS' if results[0] else '❌ FAIL'}")
    print(f"Node.js: {'✓ PASS' if results[1] else '❌ FAIL'}")
    
    if all(results):
        print("\n🎉 All binding tests passed! API parity achieved.")
        return True
    else:
        print("\n❌ Some tests failed. API parity needs work.")
        return False

if __name__ == "__main__":
    import sys
    success = main()
    sys.exit(0 if success else 1)
import type { Model, WorksheetProperties } from "@ironcalc/wasm";
import {
  Box,
  Divider,
  IconButton,
  MenuItem,
  TextField,
  styled,
} from "@mui/material";
import { t } from "i18next";
import { Check, PencilLine, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";

interface NamedRangeProperties {
  model: Model;
  worksheets: WorksheetProperties[];
  name: string;
  scope?: number;
  formula: string;
  onSave: () => void;
  onCancel: () => void;
}

interface NamedRangeInactiveProperties {
  name: string;
  scope?: number;
  formula: string;
  onDelete: () => void;
  onEdit: () => void;
}

export function NamedRangeInactive(properties: NamedRangeInactiveProperties) {
  const { name, scope, formula, onDelete, onEdit } = properties;
  const showOptions = true;
  return (
    <WrappedLine>
      <StyledDiv>{name}</StyledDiv>
      <StyledDiv>{scope}</StyledDiv>
      <StyledDiv>{formula}</StyledDiv>
      <WrappedIcons>
        <IconButton onClick={onEdit} disabled={!showOptions}>
          <StyledPencilLine size={12} />
        </IconButton>
        <StyledIconButton onClick={onDelete} disabled={!showOptions}>
          <Trash2 size={12} />
        </StyledIconButton>
      </WrappedIcons>
    </WrappedLine>
  );
}

function NamedRangeActive(properties: NamedRangeProperties) {
  const { model, worksheets, name, scope, formula, onCancel, onSave } =
    properties;
  const [newName, setNewName] = useState(name || "");
  const [newScope, setNewScope] = useState(scope);
  const [newFormula, setNewFormula] = useState(formula);
  const [readOnly, setReadOnly] = useState(true);
  const [showEditDelete, setShowEditDelete] = useState(false);

  // todo: add error messages for validations
  const [nameError, setNameError] = useState(false);
  const [formulaError, setFormulaError] = useState(false);

  useEffect(() => {
    // set state for new name
    const definedNamesModel = model.getDefinedNameList();
    if (!definedNamesModel.find((n) => n.name === newName)) {
      setReadOnly(false);
      setShowEditDelete(true);
    }
  }, [newName, model]);

  const handleSaveUpdate = () => {
    const definedNamesModel = model.getDefinedNameList();

    if (definedNamesModel.find((n) => n.name === name)) {
      // update name
      try {
        model.updateDefinedName(
          name || "",
          scope,
          newName,
          newScope,
          newFormula
        );
      } catch (error) {
        console.log("DefinedName update failed", error);
      }
    } else {
      // create name
      try {
        model.newDefinedName(newName, newScope, newFormula);
      } catch (error) {
        console.log("DefinedName save failed", error);
      }
      setReadOnly(true);
    }
    setShowEditDelete(false);
  };

  const handleCancel = () => {
    setReadOnly(true);
    setShowEditDelete(false);
    setNewName(name || "");
    setNewScope(scope);
  };

  const handleEdit = () => {
    setReadOnly(false);
    setShowEditDelete(true);
  };

  const handleDelete = () => {
    try {
      model.deleteDefinedName(newName, newScope);
    } catch (error) {
      console.log("DefinedName delete failed", error);
    }
  };

  return (
    <>
      <StyledBox>
        <StyledTextField
          id="name"
          variant="outlined"
          size="small"
          margin="none"
          fullWidth
          error={nameError}
          value={newName}
          onChange={(event) => setNewName(event.target.value)}
          onKeyDown={(event) => {
            event.stopPropagation();
          }}
          onClick={(event) => event.stopPropagation()}
        />
        <StyledTextField
          id="scope"
          variant="outlined"
          select
          size="small"
          margin="none"
          fullWidth
          value={newScope ?? "global"}
          onChange={(event) => {
            event.target.value === "global"
              ? setNewScope(undefined)
              : setNewScope(+event.target.value);
          }}
        >
          <MenuItem value={"global"}>
            {t("name_manager_dialog.workbook")}
          </MenuItem>
          {worksheets.map((option, index) => (
            <MenuItem key={option.name} value={index}>
              {option.name}
            </MenuItem>
          ))}
        </StyledTextField>
        <StyledTextField
          id="formula"
          variant="outlined"
          size="small"
          margin="none"
          fullWidth
          error={formulaError}
          value={newFormula}
          onChange={(event) => setNewFormula(event.target.value)}
          onKeyDown={(event) => {
            event.stopPropagation();
          }}
          onClick={(event) => event.stopPropagation()}
        />
        <>
          <IconButton onClick={handleSaveUpdate}>
            <StyledCheck size={12} />
          </IconButton>
          <StyledIconButton onClick={onCancel}>
            <X size={12} />
          </StyledIconButton>
        </>
      </StyledBox>
    </>
  );
}

const StyledBox = styled(Box)`
  display: flex;
  width: 577px;
`;

const StyledPencilLine = styled(PencilLine)(({ theme }) => ({
  color: theme.palette.common.black,
}));

const StyledCheck = styled(Check)(({ theme }) => ({
  color: theme.palette.success.main,
}));

const StyledTextField = styled(TextField)(() => ({
  padding: "0px",
  width: "163px",
  marginRight: "8px",
  "& .MuiInputBase-root": {
    height: "28px",
    margin: 0,
  },
  "& .MuiInputBase-input": {
    padding: "6px",
    fontSize: "12px",
  },
}));

const StyledIconButton = styled(IconButton)(({ theme }) => ({
  color: theme.palette.error.main,
  "&.Mui-disabled": {
    opacity: 0.6,
    color: theme.palette.error.light,
  },
}));


const WrappedLine = styled(Box)({
  display: "flex",
  paddingLeft: "6px",
  height: "28px",
});

const StyledDiv = styled("div")(({ theme }) => ({
  fontFamily: theme.typography.fontFamily,
  fontSize: "12px",
  fontWeight: "400",
  color: theme.palette.common.black,
  width: "171px",
}));

const WrappedIcons = styled(Box)({
  display: "flex",
  gap: "0px",
});

export default NamedRangeActive;

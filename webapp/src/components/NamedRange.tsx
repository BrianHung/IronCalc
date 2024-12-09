import type { DefinedName, Model, WorksheetProperties } from "@ironcalc/wasm";
import {
  Box,
  Divider,
  IconButton,
  MenuItem,
  TextField,
  styled,
  Box,
  Divider,
  IconButton,
  MenuItem,
  TextField,
  styled,
} from "@mui/material";
import { Trash2 } from "lucide-react";
import { useEffect, useState } from "react";

type NamedRangeProperties = {
  name?: string;
  scope?: number;
  formula: string;
  model: Model;
  worksheets: WorksheetProperties[];
  onChange: (
    field: keyof DefinedName,
    value: string | number | undefined,
  ) => void;
  onDelete?: (name: string, scope: number | undefined) => void;
  canDelete: boolean;
  onUpdate?: (
    name: string,
    scope: number | undefined,
    newName: string,
    newScope: number | undefined,
    newFormula: string,
  ) => void;
};

function NamedRange(props: NamedRangeProperties) {
  const [name, setName] = useState(props.name || "");
  const [scope, setScope] = useState(props.scope || undefined);
  const [formula, setFormula] = useState(props.formula);
  const [nameError, setNameError] = useState(false);
  const [formulaError, setFormulaError] = useState(false);

  const handleChange = (
    field: keyof DefinedName,
    value: string | number | undefined,
  ) => {
    if (field === "name") {
      setName(value as string);
      props.onChange("name", value);
    }
    if (field === "scope") {
      setScope(value as number | undefined);
      props.onChange("scope", value);
    }
    if (field === "formula") {
      setFormula(value as string);
      props.onChange("formula", value);
    }
  };

  useEffect(() => {
    // send initial formula value to parent
    handleChange("formula", formula);
  }, []);

  const handleDelete = () => {
    props.onDelete?.(name, scope);
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
          value={name}
          onChange={(event) => handleChange("name", event.target.value)}
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
          value={scope ?? "Workbook (Global)"}
          onChange={(event) =>
            handleChange(
              "scope",
              event.target.value === "Workbook (Global)"
                ? undefined
                : event.target.value,
            )
          }
        >
          <MenuItem value="Workbook (Global)">Workbook (Global)</MenuItem>
          {props.worksheets.map((option, index) => (
            <MenuItem key={option.sheet_id} value={index}>
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
          value={formula}
          onChange={(event) => handleChange("formula", event.target.value)}
          onKeyDown={(event) => {
            event.stopPropagation();
          }}
          onClick={(event) => event.stopPropagation()}
        />
        <StyledIconButton onClick={handleDelete} disabled={!props.canDelete}>
          <Trash2 size={12} />
        </StyledIconButton>
      </StyledBox>
      <Divider />
    </>
  );
}

const StyledBox = styled(Box)`
display: flex;
gap: 12px;
width: 577px;
`;

const StyledTextField = styled(TextField)(() => ({
  "& .MuiInputBase-root": {
    height: "28px",
    margin: 0,
  },
  "& .MuiInputBase-root": {
    height: "28px",
    margin: 0,
  },
}));

const StyledIconButton = styled(IconButton)(({ theme }) => ({
  color: theme.palette.error.main,
  "&.Mui-disabled": {
    opacity: 0.6,
    color: theme.palette.error.light,
  },
  color: theme.palette.error.main,
  "&.Mui-disabled": {
    opacity: 0.6,
    color: theme.palette.error.light,
  },
}));

export default NamedRange;

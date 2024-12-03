import type { Model, WorksheetProperties } from "@ironcalc/wasm";
import { Box, IconButton, MenuItem, TextField, styled } from "@mui/material";
import { Trash2 } from "lucide-react";

export type NamedRangeObject = {
  id: string;
  name: string;
  scope: string;
  range: string;
};

type NamedRangeProperties = {
  name: string;
  scope: string;
  range: string;
  model: Model;
  worksheets: WorksheetProperties[];
  // update namedRange in model
  onChange: (field: string, value: string) => void;
};

function NamedRange(props: NamedRangeProperties) {
  // define onChange in parent for updating the model and values

  const handleDelete = () => {
    // update model
    console.log("deleted named range");
  };

  return (
    <StyledBox>
      <TextField
        variant="outlined"
        size="small"
        margin="normal"
        fullWidth
        value={props.name}
        onChange={(event) =>
          props.onChange("name", event.target.value)
        }
        onKeyDown={(event) => {
          event.stopPropagation();
        }}
        onClick={(event) => event.stopPropagation()}
      />
      <TextField
        variant="outlined"
        select
        defaultValue="Workbook"
        size="small"
        margin="normal"
        fullWidth
        value={props.scope}
        onChange={(event) =>
          props.onChange("scope", event.target.value)
        }
      >
        {props.worksheets.map((option) => (
          <MenuItem key={option.sheet_id} value={option.name}>
            {option.name}
          </MenuItem>
        ))}
      </TextField>
      <TextField
        variant="outlined"
        size="small"
        margin="normal"
        fullWidth
        value={props.range}
        onChange={(event) =>
          props.onChange("range", event.target.value)
        }
        onKeyDown={(event) => {
          event.stopPropagation();
        }}
        onClick={(event) => event.stopPropagation()}
      />
      {/* remove round hover animation  */}
      <IconButton onClick={handleDelete}>
        <Trash2 size={16} absoluteStrokeWidth />
      </IconButton>
    </StyledBox>
  );
}

const StyledBox = styled(Box)`
display: flex;
gap: 15px;
`;

export default NamedRange;

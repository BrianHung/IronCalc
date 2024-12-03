import type { Model } from "@ironcalc/wasm";
import {
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  IconButton,
  styled,
} from "@mui/material";
import { t } from "i18next";
import { BookOpen, X } from "lucide-react";
import { useState } from "react";
import NamedRange from "./NamedRange";
import type { NamedRangeObject } from "./NamedRange";

type NameManagerDialogProperties = {
  onClose: () => void;
  onSave: () => void;
  open: boolean;
  model: Model;
};

function NameManagerDialog(props: NameManagerDialogProperties) {

  const handleClose = () => {
    props.onClose();
  };

  // update child component values in model
  const handleSave = () => {
    props.onSave(); // => onNamedRangesUpdate from toolbar
    props.onClose();
  };

  //! Why are fields editable only while clicking them?
  // update child component values in UI
  const handleChange = (id: string, field: string, value: string) => {
    console.log("change:", id, field, value);

    // previous array elements, plus updated value
    // setNamedRangesLocal((prev) =>
    //   prev.map((namedRange) =>
    //     namedRange.id === id ? { ...namedRange, [field]: value } : namedRange,
    //   ),
    // );
  };

  const nameList = props.model.getDefinedNameList();

  return (
    <StyledDialog
      open={props.open}
      onClose={props.onClose}
      maxWidth="md"
      fullWidth
      scroll="paper"
    >
      <StyledDialogTitle>
        Named Ranges
        <IconButton onClick={handleClose}>
          <X size={16} />
        </IconButton>
      </StyledDialogTitle>
      <DialogContent dividers>
        {nameList.map((e) => (
          <NamedRange
            worksheets={props.model.getWorksheetsProperties()}
            name={e.name}
            scope={e.scope}
            range={e.formula}
            key={`${e.name}-${e.scope}`}
            onChange={handleChange}
            model={props.model}
          />
        ))}
      </DialogContent>
      <StyledDialogActions>
        <Box display="flex" alignItems="center">
          <BookOpen
            color="grey"
            style={{ width: 16, height: 16, marginLeft: 12, marginRight: 8 }}
          />
          <span style={{ fontSize: "12px", fontFamily: "Inter" }}>
            {t("name_manager_dialog.help")}
          </span>
        </Box>
        <Box display="flex" gap="10px">
          <Button onClick={handleClose} variant="contained" color="info">
            Cancel
          </Button>
          <Button onClick={handleSave} variant="contained">
            Save changes
          </Button>
        </Box>
      </StyledDialogActions>
    </StyledDialog>
  );
}

const StyledDialog = styled(Dialog)(() => ({
  "& .MuiPaper-root": {
    maxHeight: "60%",
    minHeight: "40%",
  },
}));

// font-weight: 600 is too bold compared to design, should be between 500 & 600
const StyledDialogTitle = styled(DialogTitle)`
font-size: 14px;
font-weight: 600;
display: flex;
align-items: center;
justify-content: space-between;
`;

const StyledDialogActions = styled(DialogActions)`
height: 40px;
display: flex;
align-items: center;
justify-content: space-between;
font-size: 12px;
color: #757575;
`;

export default NameManagerDialog;

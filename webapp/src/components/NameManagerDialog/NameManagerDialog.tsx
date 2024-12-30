import type { Model } from "@ironcalc/wasm";
import {
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  IconButton,
  Stack,
  styled,
} from "@mui/material";
import { t } from "i18next";
import { BookOpen, Plus, X } from "lucide-react";
import { useState } from "react";
import { getFullRangeToString } from "../util";
import NamedRangeActive, { NamedRangeInactive } from "./NamedRange";

interface NameManagerDialogProperties {
  onClose: () => void;
  open: boolean;
  model: Model;
  onNamesChanged: () => void;
}

function NameManagerDialog(properties: NameManagerDialogProperties) {
  const { onClose, open, model, onNamesChanged } = properties;

  // If editingNameIndex is -1, then we are adding a new name
  // If editingNameIndex is -2, then we are not editing any name
  // If editingNameIndex is a positive number, then we are editing that index
  const [editingNameIndex, setEditingNameIndex] = useState(-2);

  const handleNewName = () => {
    setEditingNameIndex(-1);
  };

  const handleDelete = () => {
    onNamesChanged();
  };

  const formatFormula = (): string => {
    const worksheets = model.getWorksheetsProperties();
    const selectedView = model.getSelectedView();

    return getFullRangeToString(selectedView, worksheets);
  };

  const worksheets = model.getWorksheetsProperties();
  const definedNameList = model.getDefinedNameList();

  return (
    <StyledDialog open={open} onClose={onClose} maxWidth={false} scroll="paper">
      <StyledDialogTitle>
        {t("name_manager_dialog.title")}
        <IconButton onClick={onClose}>
          <X size={16} />
        </IconButton>
      </StyledDialogTitle>
      <StyledDialogContent dividers>
        <StyledRangesHeader>
          <StyledBox>{t("name_manager_dialog.name")}</StyledBox>
          <StyledBox>{t("name_manager_dialog.range")}</StyledBox>
          <StyledBox>{t("name_manager_dialog.scope")}</StyledBox>
        </StyledRangesHeader>
        <NameLisWrapper>
          {definedNameList.map((definedName, index) => {
            if (index === editingNameIndex) {
              return (
                <NamedRangeActive
                  model={model}
                  worksheets={worksheets}
                  name={definedName.name}
                  scope={definedName.scope}
                  formula={definedName.formula}
                  key={definedName.name}
                  onSave={onNamesChanged}
                  onCancel={() => setEditingNameIndex(-2)}
                />
              );
            }
            return (
              <NamedRangeInactive
                name={definedName.name}
                scope={definedName.scope}
                formula={definedName.formula}
                key={definedName.name}
                onEdit={() => setEditingNameIndex(index)}
                onDelete={handleDelete}
              />
            );
          })}
        </NameLisWrapper>
        {editingNameIndex === -1 && (
          <NamedRangeActive
          model={model}
          worksheets={worksheets}
          name={"Name1"}
          scope={0}
          formula={formatFormula()}
          onSave={onNamesChanged}
          onCancel={() => setEditingNameIndex(-2)}
        />)
      }
      </StyledDialogContent>
      <StyledDialogActions>
        <Box display="flex" alignItems="center" gap={"8px"}>
          <BookOpen color="grey" size={16} />
          <span style={{ fontSize: "12px", fontFamily: "Inter" }}>
            {t("name_manager_dialog.help")}
          </span>
        </Box>
        <Button
          onClick={handleNewName}
          variant="contained"
          disableElevation
          sx={{ textTransform: "none" }}
          startIcon={<Plus size={16} />}
          disabled={editingNameIndex > -2}
        >
          {t("name_manager_dialog.new")}
        </Button>
      </StyledDialogActions>
    </StyledDialog>
  );
}

const NameLisWrapper = styled(Stack)`
  overflow-y: auto;
`;

const StyledBox = styled("div")`
  width: 171px;
`;

const StyledDialog = styled(Dialog)(() => ({
  "& .MuiPaper-root": {
    height: "380px",
    minWidth: "620px",
  },
}));

const StyledDialogTitle = styled(DialogTitle)`
  padding: 12px 20px;
  font-size: 14px;
  font-weight: 600;
  display: flex;
  align-items: center;
  justify-content: space-between;
`;

const StyledDialogContent = styled(DialogContent)`
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 20px 12px 20px 20px;
`;

const StyledRangesHeader = styled(Box)(({ theme }) => ({
  display: "flex",
  paddingLeft: "6px",
  fontFamily: theme.typography.fontFamily,
  fontSize: "12px",
  fontWeight: "700",
  color: theme.palette.info.main,
}));

const StyledDialogActions = styled(DialogActions)`
  padding: 12px 20px;
  height: 40px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  color: #757575;
`;

export default NameManagerDialog;

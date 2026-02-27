package notebook

import (
	"github.com/scienceol/osdl/pkg/common/uuid"
	"github.com/scienceol/osdl/pkg/repo/model"
)

type NoteBookGroupData struct {
	Group          *model.NotebookGroup
	SampleMaterial map[uuid.UUID]uuid.UUID
	Params         map[int64]*model.NotebookParam
}

type NotebookData struct {
	NotebookID       int64
	NotebookGroupIDs []int64
	NotebookGroupMap map[int64]*NoteBookGroupData
}

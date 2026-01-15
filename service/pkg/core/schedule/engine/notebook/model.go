package notebook

import model "github.com/scienceol/opensdl/service/pkg/model"

type NoteBookGroupData struct {
	Group  *model.NotebookGroup
	Params map[int64]*model.NotebookParam
}

type NotebookData struct {
	NotebookID       int64
	NotebookGroupIDs []int64                      // 保持顺序
	NotebookGroupMap map[int64]*NoteBookGroupData // 索引
}

package notebook

import (
	// 外部依赖
	"context"
)

type Service interface {
	// QueryNotebookList 查询 notebook 列表
	QueryNotebookList(ctx context.Context, req *QueryNotebookReq) (*QueryNotebookResp, error)
	// CreateNotebook 创建 notebook
	CreateNotebook(ctx context.Context, req *CreateNotebookReq) (*CreateNotebookResp, error)
	// DeleteNotebook 删除 notebook
	DeleteNotebook(ctx context.Context, req *DeleteNotebookReq) error
	// 获取工作流 schema
	NotebookSchema(ctx context.Context, req *NotebookSchemaReq) (*NotebookSchemaResp, error)
	// 运行实验记录本
	NotebookDetail(ctx context.Context, req *NotebookDetailReq) (*NotebookDetailResp, error)
	// 创建样品
	CreateSample(ctx context.Context, req *SampleReq) (*SampleResp, error)
	// GetNotebookBySample 根据样品获取 notebook
	GetNotebookBySample(ctx context.Context, req *GetNotebookBySampleReq) (*GetNotebookBySampleResp, error)
}

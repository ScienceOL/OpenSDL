package repo

import (
	// 外部依赖
	"context"

	common "github.com/scienceol/opensdl/service/pkg/common"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	nbCore "github.com/scienceol/opensdl/service/pkg/core/notebook"
	model "github.com/scienceol/opensdl/service/pkg/model"
)

type NotebookRepo interface {
	IDOrUUIDTranslate
	// GetNotebookList 查询 notebook 列表
	GetNotebookList(ctx context.Context, labID int64, req *nbCore.QueryNotebookReq) (*common.PageResp[[]*model.Notebook], error)
	// GetNotebookByUUID 根据 UUID 获取 notebook
	GetNotebookByUUID(ctx context.Context, notebookUUID uuid.UUID) (*model.Notebook, error)
	// GetNotebookGroupList 查询 notebook_group 列表
	GetNotebookGroupList(ctx context.Context, notebookID int64, req *common.PageReq) (*common.PageMoreResp[[]*model.NotebookGroup], error)
	// CreateNotebook 创建 notebook
	CreateNotebook(ctx context.Context, data *model.Notebook) error
	// CreateNotebookGroup 创建 notebook_group
	CreateNotebookGroup(ctx context.Context, data *model.NotebookGroup) error
	// DeleteNotebook 删除 notebook
	DeleteNotebook(ctx context.Context, notebookID int64) error
	// DeleteNotebookGroup 删除 notebook_group（软删除）
	DeleteNotebookGroup(ctx context.Context, notebookGroupID int64) error
	// GetNotebookGroupByUUID 根据 UUID 获取 notebook_group
	GetNotebookGroupByUUID(ctx context.Context, notebookGroupUUID uuid.UUID) (*model.NotebookGroup, error)
	// GetNotebookGroups 获取指定 notebook 下的所有未删除分组
	GetNotebookGroups(ctx context.Context, notebookID int64) ([]*model.NotebookGroup, error)
	// GetNotebookParamsByGroupIDs 根据分组 ID 列表获取参数记录
	GetNotebookParamsByGroupIDs(ctx context.Context, groupIDs []int64) ([]*model.NotebookParam, error)
	// 创建样品
	CreateSample(ctx context.Context, datas []*model.Sample) error
	// GetDistinctJobIDsBySampleIDs 根据 sample_id 列表获取唯一的 job_id 列表
	GetDistinctJobIDsBySampleIDs(ctx context.Context, sampleIDs []int64) ([]int64, error)
	// GetDistinctNotebookGroupIDsByTaskIDs 根据 workflow_task_id 列表获取唯一的 notebook_group_id 列表
	GetDistinctNotebookGroupIDsByTaskIDs(ctx context.Context, taskIDs []int64) ([]int64, error)
	// GetDistinctNotebookIDsByGroupIDs 根据 notebook_group_id 列表获取唯一的 notebook_id 列表
	GetDistinctNotebookIDsByGroupIDs(ctx context.Context, groupIDs []int64) ([]int64, error)
	// GetSampleIDsByName 根据 sample name 获取 sample_id 列表
	GetSampleIDsByName(ctx context.Context, name string) ([]int64, error)
}

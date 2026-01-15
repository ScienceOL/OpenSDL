package notebook

import (
	// 外部依赖
	"context"
	"errors"
	"strings"
	"time"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	nbCore "github.com/scienceol/opensdl/service/pkg/core/notebook"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	model "github.com/scienceol/opensdl/service/pkg/model"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	gorm "gorm.io/gorm"
)

type notebookImpl struct {
	repo.IDOrUUIDTranslate
}

func New() repo.NotebookRepo {
	return &notebookImpl{
		IDOrUUIDTranslate: repo.NewBaseDB(),
	}
}

// GetNotebookList 查询 notebook 列表
func (n *notebookImpl) GetNotebookList(ctx context.Context, labID int64, req *nbCore.QueryNotebookReq) (*common.PageResp[[]*model.Notebook], error) {
	req.PageReq.Normalize()

	query := n.DBWithContext(ctx).Model(&model.Notebook{})

	// 过滤已删除的记录（软删除）
	query = query.Where("status != ?", model.NotebookStatusDeleted)

	query = query.Where("lab_id = ?", labID)

	// 根据搜索关键词过滤（按名称搜索）
	if req.Search != "" {
		query = query.Where("name ILIKE ?", "%"+req.Search+"%")
	}

	// 根据状态过滤
	if len(req.Status) > 0 {
		query = query.Where("status IN ?", req.Status)
	}

	// 计算总数
	var total int64
	if err := query.Count(&total).Error; err != nil {
		logger.Errorf(ctx, "GetNotebookList count err: %+v", err)
		return nil, code.QueryRecordErr.WithMsg(err.Error())
	}

	// 构建排序逻辑
	orderClauses := make([]string, 0, 3)
	if req.SubmitTime != nil {
		orderClauses = append(orderClauses, "submit_time "+string(*req.SubmitTime))
	}
	if req.ScheduleStartTime != nil {
		orderClauses = append(orderClauses, "start_time "+string(*req.ScheduleStartTime))
	}
	if req.ScheduleFinishedTime != nil {
		orderClauses = append(orderClauses, "finished_time "+string(*req.ScheduleFinishedTime))
	}

	// 如果没有指定排序，则默认按 ID 降序
	if len(orderClauses) == 0 {
		query = query.Order("id desc")
	} else {
		query = query.Order(strings.Join(orderClauses, ", "))
	}

	// 查询数据
	var notebooks []*model.Notebook
	offset := req.Offest()
	if err := query.
		Offset(offset).
		Limit(req.PageSize).
		Find(&notebooks).Error; err != nil {
		logger.Errorf(ctx, "GetNotebookList query err: %+v", err)
		return nil, code.QueryRecordErr.WithMsg(err.Error())
	}

	return &common.PageResp[[]*model.Notebook]{
		Total:    total,
		Page:     req.Page,
		PageSize: req.PageSize,
		Data:     notebooks,
	}, nil
}

// GetNotebookGroupByUUID 根据 UUID 获取 notebook_group
func (n *notebookImpl) GetNotebookGroupByUUID(ctx context.Context, notebookGroupUUID uuid.UUID) (*model.NotebookGroup, error) {
	notebookGroup := &model.NotebookGroup{}
	if err := n.DBWithContext(ctx).
		Where("uuid = ?", notebookGroupUUID).
		Where("status != ?", model.NotebookStatusDeleted).
		Take(notebookGroup).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, code.RecordNotFound
		}
		logger.Errorf(ctx, "GetNotebookGroupByUUID err: %+v", err)
		return nil, code.QueryRecordErr.WithMsg(err.Error())
	}
	return notebookGroup, nil
}

// GetNotebookGroups 获取指定 notebook 下的所有未删除分组
func (n *notebookImpl) GetNotebookGroups(ctx context.Context, notebookID int64) ([]*model.NotebookGroup, error) {
	groups := make([]*model.NotebookGroup, 0)
	if notebookID == 0 {
		return groups, nil
	}

	if err := n.DBWithContext(ctx).
		Where("notebook_id = ?", notebookID).
		Where("status != ?", model.NotebookStatusDeleted).
		Order("id ASC").
		Find(&groups).Error; err != nil {
		logger.Errorf(ctx, "GetNotebookGroups err: %+v", err)
		return nil, code.QueryRecordErr.WithMsg(err.Error())
	}

	return groups, nil
}

// GetNotebookParamsByGroupIDs 根据分组 ID 列表获取参数记录
func (n *notebookImpl) GetNotebookParamsByGroupIDs(ctx context.Context, groupIDs []int64) ([]*model.NotebookParam, error) {
	params := make([]*model.NotebookParam, 0)
	if len(groupIDs) == 0 {
		return params, nil
	}

	if err := n.DBWithContext(ctx).
		Where("notebook_group_id IN ?", groupIDs).
		Order("id ASC").
		Find(&params).Error; err != nil {
		logger.Errorf(ctx, "GetNotebookParamsByGroupIDs err: %+v", err)
		return nil, code.QueryRecordErr.WithMsg(err.Error())
	}

	return params, nil
}

// DeleteNotebookGroup 删除 notebook_group（软删除：将 Status 设置为 deleted）
func (n *notebookImpl) DeleteNotebookGroup(ctx context.Context, notebookGroupID int64) error {
	// 软删除：将 notebook_group 的 status 更新为 deleted
	if err := n.DBWithContext(ctx).Model(&model.NotebookGroup{}).
		Where("id = ?", notebookGroupID).
		Update("status", model.NotebookStatusDeleted).Error; err != nil {
		logger.Errorf(ctx, "DeleteNotebookGroup update status err: %+v", err)
		return code.UpdateDataErr.WithMsg(err.Error())
	}

	return nil
}

// GetNotebookByUUID 根据 UUID 获取 notebook
func (n *notebookImpl) GetNotebookByUUID(ctx context.Context, notebookUUID uuid.UUID) (*model.Notebook, error) {
	notebook := &model.Notebook{}
	if err := n.DBWithContext(ctx).
		Where("uuid = ? AND status != ?", notebookUUID, model.NotebookStatusDeleted).
		Take(notebook).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, code.RecordNotFound.WithMsgf("notebook uuid: %s", notebookUUID)
		}
		logger.Errorf(ctx, "GetNotebookByUUID fail uuid: %+v, error: %+v", notebookUUID, err)
		return nil, code.QueryRecordErr.WithMsg(err.Error())
	}
	return notebook, nil
}

// GetNotebookGroupList 查询 notebook_group 列表
func (n *notebookImpl) GetNotebookGroupList(ctx context.Context, notebookID int64, req *common.PageReq) (*common.PageMoreResp[[]*model.NotebookGroup], error) {
	req.Normalize()

	query := n.DBWithContext(ctx).Model(&model.NotebookGroup{})

	// 过滤已删除的记录（软删除）
	query = query.Where("status != ?", model.NotebookStatusDeleted)

	// 根据 notebook_id 过滤
	if notebookID > 0 {
		query = query.Where("notebook_id = ?", notebookID)
	}

	// 计算总数
	var total int64
	if err := query.Count(&total).Error; err != nil {
		logger.Errorf(ctx, "GetNotebookGroupList count err: %+v", err)
		return nil, code.QueryRecordErr.WithMsg(err.Error())
	}

	// 查询数据
	var groups []*model.NotebookGroup
	offset := req.Offest()
	if err := query.Order("id desc").
		Offset(offset).
		Limit(req.PageSize).
		Find(&groups).Error; err != nil {
		logger.Errorf(ctx, "GetNotebookGroupList query err: %+v", err)
		return nil, code.QueryRecordErr.WithMsg(err.Error())
	}

	hasMore := int64(req.Page*req.PageSize) < total
	return &common.PageMoreResp[[]*model.NotebookGroup]{
		HasMore:  hasMore,
		Page:     req.Page,
		PageSize: req.PageSize,
		Data:     groups,
	}, nil
}

// CreateNotebook 创建 notebook
func (n *notebookImpl) CreateNotebook(ctx context.Context, data *model.Notebook) error {
	if err := n.DBWithContext(ctx).Create(data).Error; err != nil {
		logger.Errorf(ctx, "CreateNotebook err: %+v", err)
		return code.CreateDataErr.WithMsg(err.Error())
	}
	return nil
}

// CreateNotebookGroup 创建 notebook_group
func (n *notebookImpl) CreateNotebookGroup(ctx context.Context, data *model.NotebookGroup) error {
	if err := n.DBWithContext(ctx).Create(data).Error; err != nil {
		logger.Errorf(ctx, "CreateNotebookGroup err: %+v", err)
		return code.CreateDataErr.WithMsg(err.Error())
	}
	return nil
}

// DeleteNotebook 删除 notebook（软删除：将 Status 设置为 deleted）
func (n *notebookImpl) DeleteNotebook(ctx context.Context, notebookID int64) error {
	now := time.Now()
	// 软删除：将 notebook 的 status 更新为 deleted，并设置 deleted_at
	if err := n.DBWithContext(ctx).Model(&model.Notebook{}).
		Where("id = ?", notebookID).
		Updates(map[string]any{
			"status":     model.NotebookStatusDeleted,
			"deleted_at": now,
		}).Error; err != nil {
		logger.Errorf(ctx, "DeleteNotebook update status err: %+v", err)
		return code.UpdateDataErr.WithMsg(err.Error())
	}

	// 同时软删除关联的 notebook_group（将它们的 status 也设置为 deleted）
	if err := n.DBWithContext(ctx).Model(&model.NotebookGroup{}).
		Where("notebook_id = ?", notebookID).
		Update("status", model.NotebookStatusDeleted).Error; err != nil {
		logger.Errorf(ctx, "DeleteNotebook update groups status err: %+v", err)
		return code.UpdateDataErr.WithMsg(err.Error())
	}

	return nil
}

func (n *notebookImpl) CreateSample(ctx context.Context, datas []*model.Sample) error {
	if err := n.DBWithContext(ctx).Create(datas).Error; err != nil {
		logger.Errorf(ctx, "notebookImpl.CreateSample fail err: %+v", err)
		return code.CreateDataErr
	}
	return nil
}

// GetDistinctJobIDsBySampleIDs 根据 sample_id 列表获取唯一的 job_id 列表
func (n *notebookImpl) GetDistinctJobIDsBySampleIDs(ctx context.Context, sampleIDs []int64) ([]int64, error) {
	if len(sampleIDs) == 0 {
		return []int64{}, nil
	}

	var jobIDs []int64
	if err := n.DBWithContext(ctx).
		Model(&model.WorkflowNodeJobSample{}).
		Where("sample_id IN ?", sampleIDs).
		Distinct("job_id").
		Pluck("job_id", &jobIDs).Error; err != nil {
		logger.Errorf(ctx, "GetDistinctJobIDsBySampleIDs err: %+v", err)
		return nil, code.QueryRecordErr.WithMsg(err.Error())
	}

	return jobIDs, nil
}

// GetDistinctNotebookGroupIDsByTaskIDs 根据 workflow_task_id 列表获取唯一的 notebook_group_id 列表
func (n *notebookImpl) GetDistinctNotebookGroupIDsByTaskIDs(ctx context.Context, taskIDs []int64) ([]int64, error) {
	if len(taskIDs) == 0 {
		return []int64{}, nil
	}

	var notebookGroupIDs []int64
	if err := n.DBWithContext(ctx).
		Model(&model.WorkflowTask{}).
		Where("id IN ?", taskIDs).
		Distinct("notebook_group_id").
		Pluck("notebook_group_id", &notebookGroupIDs).Error; err != nil {
		logger.Errorf(ctx, "GetDistinctNotebookGroupIDsByTaskIDs err: %+v", err)
		return nil, code.QueryRecordErr.WithMsg(err.Error())
	}

	return notebookGroupIDs, nil
}

// GetDistinctNotebookIDsByGroupIDs 根据 notebook_group_id 列表获取唯一的 notebook_id 列表
func (n *notebookImpl) GetDistinctNotebookIDsByGroupIDs(ctx context.Context, groupIDs []int64) ([]int64, error) {
	if len(groupIDs) == 0 {
		return []int64{}, nil
	}

	var notebookIDs []int64
	if err := n.DBWithContext(ctx).
		Model(&model.NotebookGroup{}).
		Where("id IN ?", groupIDs).
		Distinct("notebook_id").
		Pluck("notebook_id", &notebookIDs).Error; err != nil {
		logger.Errorf(ctx, "GetDistinctNotebookIDsByGroupIDs err: %+v", err)
		return nil, code.QueryRecordErr.WithMsg(err.Error())
	}

	return notebookIDs, nil
}

// GetSampleIDsByName 根据 sample name 获取 sample_id 列表
func (n *notebookImpl) GetSampleIDsByName(ctx context.Context, name string) ([]int64, error) {
	var sampleIDs []int64
	if err := n.DBWithContext(ctx).
		Model(&model.Sample{}).
		Where("name = ?", name).
		Pluck("id", &sampleIDs).Error; err != nil {
		logger.Errorf(ctx, "GetSampleIDsByName err: %+v", err)
		return nil, code.QueryRecordErr.WithMsg(err.Error())
	}

	return sampleIDs, nil
}

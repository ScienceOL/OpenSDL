package environment

import (
	"context"

	"github.com/scienceol/osdl/pkg/common"
	"github.com/scienceol/osdl/pkg/common/uuid"
	"github.com/scienceol/osdl/pkg/middleware/db"
	"github.com/scienceol/osdl/pkg/repo"
	"github.com/scienceol/osdl/pkg/repo/model"
)

type envImpl struct {
	*db.Datastore
}

func New() repo.LaboratoryRepo {
	return &envImpl{Datastore: db.DB()}
}

func (e *envImpl) CreateLaboratoryEnv(ctx context.Context, data *model.Laboratory) error {
	return e.DBWithContext(ctx).Create(data).Error
}

func (e *envImpl) UpdateLaboratoryEnv(ctx context.Context, data *model.Laboratory) error {
	return e.DBWithContext(ctx).Model(data).Updates(data).Error
}

func (e *envImpl) GetLabByUUID(ctx context.Context, UUID uuid.UUID, selectKeys ...string) (*model.Laboratory, error) {
	data := &model.Laboratory{}
	query := e.DBWithContext(ctx).Where("uuid = ?", UUID)
	if len(selectKeys) != 0 {
		query = query.Select(selectKeys)
	}
	err := query.First(data).Error
	return data, err
}

func (e *envImpl) GetLabByID(ctx context.Context, labID int64, selectKeys ...string) (*model.Laboratory, error) {
	data := &model.Laboratory{}
	query := e.DBWithContext(ctx).Where("id = ?", labID)
	if len(selectKeys) != 0 {
		query = query.Select(selectKeys)
	}
	err := query.First(data).Error
	return data, err
}

func (e *envImpl) GetLabByAkSk(ctx context.Context, accessKey string, accessSecret string) (*model.Laboratory, error) {
	data := &model.Laboratory{}
	err := e.DBWithContext(ctx).Where("access_key = ? AND access_secret = ?", accessKey, accessSecret).First(data).Error
	return data, err
}

func (e *envImpl) GetLabList(ctx context.Context, userIDs []string, req *common.PageReq) (*common.PageResp[[]*model.Laboratory], error) {
	var datas []*model.Laboratory
	var total int64
	req.Normalize()
	d := e.DBWithContext(ctx).Model(&model.Laboratory{}).Where("user_id IN ?", userIDs)
	d.Count(&total)
	err := d.Limit(req.PageSize).Offset(req.Offest()).Find(&datas).Error
	return &common.PageResp[[]*model.Laboratory]{Data: datas, Total: total, Page: req.Page, PageSize: req.PageSize}, err
}

func (e *envImpl) AddLabMember(ctx context.Context, datas ...*model.LaboratoryMember) error {
	if len(datas) == 0 {
		return nil
	}
	return e.DBWithContext(ctx).Create(&datas).Error
}

func (e *envImpl) GetLabByUserID(_ context.Context, _ *common.PageReqT[string]) (*common.PageResp[[]*model.LaboratoryMember], error) {
	return nil, nil // TODO
}

func (e *envImpl) GetLabByLabID(_ context.Context, _ *common.PageReqT[int64]) (*common.PageResp[[]*model.LaboratoryMember], error) {
	return nil, nil // TODO
}

func (e *envImpl) GetAllResourceName(_ context.Context, _ int64) []string {
	return nil // TODO
}

func (e *envImpl) UpsertResourceNodeTemplate(_ context.Context, _ []*model.ResourceNodeTemplate) error {
	return nil // TODO
}

func (e *envImpl) UpsertResourceHandleTemplate(_ context.Context, _ []*model.ResourceHandleTemplate) error {
	return nil // TODO
}

func (e *envImpl) GetResourceHandleTemplates(_ context.Context, _ []int64) (map[int64][]*model.ResourceHandleTemplate, error) {
	return nil, nil // TODO
}

func (e *envImpl) GetResourceNodeTemplates(_ context.Context, _ []int64) ([]*model.ResourceNodeTemplate, error) {
	return nil, nil // TODO
}

func (e *envImpl) GetAllResourceTemplateByLabID(_ context.Context, _ int64, _ ...string) ([]*model.ResourceNodeTemplate, error) {
	return nil, nil // TODO
}

func (e *envImpl) UpsertWorkflowNodeTemplate(_ context.Context, _ []*model.WorkflowNodeTemplate) error {
	return nil // TODO
}

func (e *envImpl) UpsertActionHandleTemplate(_ context.Context, _ []*model.WorkflowHandleTemplate) error {
	return nil // TODO
}

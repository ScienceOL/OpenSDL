package repo

import (
	"context"

	"github.com/scienceol/osdl/pkg/common"
	"github.com/scienceol/osdl/pkg/common/uuid"
	"github.com/scienceol/osdl/pkg/repo/model"
)

type LaboratoryRepo interface {
	CreateLaboratoryEnv(ctx context.Context, data *model.Laboratory) error
	UpdateLaboratoryEnv(ctx context.Context, data *model.Laboratory) error
	GetLabByUUID(ctx context.Context, UUID uuid.UUID, selectKeys ...string) (*model.Laboratory, error)
	GetLabByID(ctx context.Context, labID int64, selectKeys ...string) (*model.Laboratory, error)
	GetLabByAkSk(ctx context.Context, accessKey string, accessSecret string) (*model.Laboratory, error)
	GetLabList(ctx context.Context, userIDs []string, req *common.PageReq) (*common.PageResp[[]*model.Laboratory], error)
	AddLabMember(ctx context.Context, datas ...*model.LaboratoryMember) error
	GetLabByUserID(ctx context.Context, req *common.PageReqT[string]) (*common.PageResp[[]*model.LaboratoryMember], error)
	GetLabByLabID(ctx context.Context, req *common.PageReqT[int64]) (*common.PageResp[[]*model.LaboratoryMember], error)
	GetAllResourceName(ctx context.Context, labID int64) []string
	UpsertResourceNodeTemplate(ctx context.Context, datas []*model.ResourceNodeTemplate) error
	UpsertResourceHandleTemplate(ctx context.Context, datas []*model.ResourceHandleTemplate) error
	GetResourceHandleTemplates(ctx context.Context, resourceNodeIDs []int64) (map[int64][]*model.ResourceHandleTemplate, error)
	GetResourceNodeTemplates(ctx context.Context, ids []int64) ([]*model.ResourceNodeTemplate, error)
	GetAllResourceTemplateByLabID(ctx context.Context, labID int64, selectKeys ...string) ([]*model.ResourceNodeTemplate, error)
	UpsertWorkflowNodeTemplate(ctx context.Context, datas []*model.WorkflowNodeTemplate) error
	UpsertActionHandleTemplate(ctx context.Context, datas []*model.WorkflowHandleTemplate) error
}

package laboratory

import (
	// 外部依赖
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"strconv"
	"strings"
	"time"

	datatypes "gorm.io/datatypes"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	environment "github.com/scienceol/opensdl/service/pkg/core/environment"
	eo "github.com/scienceol/opensdl/service/pkg/core/environment"
	auth "github.com/scienceol/opensdl/service/pkg/middleware/auth"
	db "github.com/scienceol/opensdl/service/pkg/middleware/db"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	// "github.com/scienceol/studio/service/pkg/repo/bohr"
	casdoor "github.com/scienceol/opensdl/service/pkg/repo/casdoor"
	eStore "github.com/scienceol/opensdl/service/pkg/repo/environment"
	invite "github.com/scienceol/opensdl/service/pkg/repo/invite"
	model "github.com/scienceol/opensdl/service/pkg/model"
	opa "github.com/scienceol/opensdl/service/pkg/repo/opa"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

type lab struct {
	envStore      repo.LaboratoryRepo
	accountClient repo.Account
	inviteStore   repo.Invite
	policy        repo.Policy
}

func NewLab() eo.EnvService {
	return &lab{
		envStore: eStore.New(),
		accountClient: func() repo.Account {
			if config.Global().Auth.AuthSource == config.AuthBohr {
				return bohr.New()
			} else {
				return casdoor.NewCasClient()
			}
		}(),
		inviteStore: invite.New(),
		policy:      opa.NewOpaClient(),
	}
}

func (l *lab) CreateLaboratoryEnv(ctx context.Context, req *eo.LaboratoryEnvReq) (*eo.LaboratoryEnvResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return nil, code.UnLogin
	}
	var data *model.Laboratory
	ak := uuid.NewV4().String()
	sk := uuid.NewV4().String()
	err := l.envStore.ExecTx(ctx, func(txCtx context.Context) error {
		data = &model.Laboratory{
			Name:         req.Name,
			UserID:       userInfo.ID,
			Status:       model.INIT,
			AccessKey:    ak,
			AccessSecret: sk,
			Description:  req.Description,
			BaseModel: model.BaseModel{
				CreatedAt: time.Now(),
				UpdatedAt: time.Now(),
			},
		}
		if err := l.envStore.CreateLaboratoryEnv(txCtx, data); err != nil {
			return err
		}

		if err := l.envStore.AddLabMember(txCtx, &model.LaboratoryMember{
			UserID: data.UserID,
			LabID:  data.ID,
			Role:   common.Admin,
		}); err != nil {
			return err
		}

		if err := l.envStore.AddLabInnerRole(txCtx, data.UserID, data.ID); err != nil {
			return err
		}

		if err := l.envStore.UpdateLaboratoryEnv(txCtx, data); err != nil {
			return err
		}

		// 往 resource_node_template 表写入 mcp_call 数据，并创建相关的 workflow_node_template 和 workflow_handle_template
		if err := l.createMCPCallResource(txCtx, data); err != nil {
			logger.Errorf(txCtx, "CreateLabResource: failed to create mcp_call resource, err: %+v", err)
			return code.CreateDataErr.WithErr(err)
		}

		return nil
	})
	if err != nil {
		return nil, err
	}

	return &eo.LaboratoryEnvResp{
		UUID:         data.UUID,
		Name:         data.Name,
		AccessKey:    ak,
		AccessSecret: sk,
	}, nil
}

// createMCPCallResource 创建 mcp_call 资源模板及相关的工作流节点模板和句柄模板
func (l *lab) createMCPCallResource(ctx context.Context, labData *model.Laboratory) error {
	// 1. 插入 resource_node_template
	mcpResource := &model.ResourceNodeTemplate{
		Name:         "mcp_call",
		Header:       "mcp_call",
		ResourceType: "tool",
		LabID:        labData.ID,
		UserID:       labData.UserID,
		Language:     "json",  // 设置默认语言类型
		Version:      "1.0.0", // 设置默认版本
	}
	if err := l.envStore.UpsertResourceNodeTemplate(ctx, []*model.ResourceNodeTemplate{mcpResource}); err != nil {
		logger.Errorf(ctx, "UpsertResourceNodeTemplate: failed to upsert mcp_call resource, err: %+v", err)
		return code.UpdateDataErr.WithErr(err)
	}

	// 2. 查询刚插入的 resource_node_template 的 ID
	var resourceNode model.ResourceNodeTemplate
	if err := l.envStore.GetData(ctx, &resourceNode, map[string]any{
		"lab_id": labData.ID,
		"name":   "mcp_call",
	}, "id"); err != nil {
		return code.QueryRecordErr.WithErr(err)
	}

	// 3. 创建 schema JSON
	schemaJSON := map[string]any{
		"type":     "object",
		"title":    "create_protocol参数",
		"required": []string{"goal"},
		"properties": map[string]any{
			"goal": map[string]any{
				"type":     "object",
				"required": []string{"host", "path", "header", "query", "body"},
				"properties": map[string]any{
					"host":   map[string]any{"type": "string"},
					"path":   map[string]any{"type": "string"},
					"header": map[string]any{"type": "string"},
					"query":  map[string]any{"type": "string"},
					"body":   map[string]any{"type": "string"},
				},
			},
			"result":   map[string]any{},
			"feedback": map[string]any{},
		},
		"description": "创建mcp服务",
	}

	schemaBytes, _ := json.Marshal(schemaJSON)

	// 4. 插入 workflow_node_template
	workflowNode := &model.WorkflowNodeTemplate{
		LabID:          labData.ID,
		ResourceNodeID: resourceNode.ID,
		Name:           "eta_mcp",
		Schema:         datatypes.JSON(schemaBytes),
		Type:           "action", // 设置默认类型
		DisplayName:    "eta_mcp",
		NodeType:       "tool_call",
	}
	if err := l.envStore.UpsertWorkflowNodeTemplate(ctx, []*model.WorkflowNodeTemplate{workflowNode}); err != nil {
		return code.UpdateDataErr.WithErr(err)
	}

	// 5. 查询刚插入的 workflow_node_template 的 ID
	var workflowNodeTemplate model.WorkflowNodeTemplate
	if err := l.envStore.GetData(ctx, &workflowNodeTemplate, map[string]any{
		"resource_node_id": resourceNode.ID,
		"name":             "eta_mcp",
	}, "id"); err != nil {
		return code.QueryRecordErr.WithErr(err)
	}

	// 6. 插入 workflow_handle_template
	workflowHandles := []*model.WorkflowHandleTemplate{
		{
			WorkflowNodeID: workflowNodeTemplate.ID,
			HandleKey:      "ready",
			IoType:         "target",
			DisplayName:    "ready",
			Type:           "default", // 设置默认类型
		},
		{
			WorkflowNodeID: workflowNodeTemplate.ID,
			HandleKey:      "ready",
			IoType:         "source",
			DisplayName:    "ready",
			Type:           "default", // 设置默认类型
		},
		{
			WorkflowNodeID: workflowNodeTemplate.ID,
			HandleKey:      "urls",
			IoType:         "target",
			DisplayName:    "urls",
			Type:           "default", // 设置默认类型
			DataKey:        "urls",
		},
	}
	if err := l.envStore.UpsertActionHandleTemplate(ctx, workflowHandles); err != nil {
		return code.UpdateDataErr.WithErr(err)
	}

	return nil
}

func (l *lab) UpdateLaboratoryEnv(ctx context.Context, req *environment.UpdateEnvReq) (*environment.LaboratoryResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return nil, code.UnLogin
	}

	data := &model.Laboratory{
		BaseModel: model.BaseModel{
			UUID:      req.UUID,
			UpdatedAt: time.Now(),
		},
		Name:        req.Name,
		UserID:      userInfo.ID,
		Description: req.Description,
	}

	err := l.envStore.UpdateLaboratoryEnv(ctx, data)
	if err != nil {
		return nil, err
	}
	return &eo.LaboratoryResp{
		UUID:        data.UUID,
		Name:        data.Name,
		Description: data.Description,
	}, nil
}

func (l *lab) DelLab(ctx context.Context, req *eo.DelLabReq) error {
	if req.UUID.IsNil() {
		return code.ParamErr.WithMsg("lab uuid is empty")
	}
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return code.UnLogin
	}

	lab, err := l.envStore.GetLabByUUID(ctx, req.UUID)
	if err != nil {
		return err
	}

	if lab.UserID != userInfo.ID {

		if err := l.envStore.DelData(ctx, &model.UserRole{}, map[string]any{
			"lab_id":  lab.ID,
			"user_id": userInfo.ID,
		}); err != nil {
			return err
		}

		// 退出该实验室
		if err := l.envStore.DelData(ctx, &model.LaboratoryMember{}, map[string]any{
			"lab_id":  lab.ID,
			"user_id": userInfo.ID,
		}); err != nil {
			return err
		}
	} else {
		if err := l.envStore.DelData(ctx, &model.UserRole{}, map[string]any{
			"lab_id": lab.ID,
		}); err != nil {
			return err
		}

		// 自己删除实验室，清空所有成员
		if err := l.envStore.DelData(ctx, &model.LaboratoryMember{}, map[string]any{
			"lab_id": lab.ID,
		}); err != nil {
			return err
		}

		if err := l.envStore.UpdateData(ctx, &model.Laboratory{
			Status: model.DELETED,
		}, map[string]any{
			"id": lab.ID,
		}, "status"); err != nil {
			return err
		}
	}

	return nil
}

func (l *lab) LabInfo(ctx context.Context, req *eo.LabInfoReq) (*eo.LabInfoResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return nil, code.UnLogin
	}

	if req.UUID.IsNil() {
		return nil, code.ParamErr
	}

	lab := &model.Laboratory{}
	var err error

	if req.Type == eo.LABAK {
		err = l.envStore.GetData(ctx, lab, map[string]any{
			"access_key": req.UUID,
		})
	} else {
		lab, err = l.envStore.GetLabByUUID(ctx, req.UUID)
	}

	if err != nil {
		return nil, err
	}

	resp := &eo.LabInfoResp{
		UUID:         uuid.UUID{},
		Name:         "",
		UserID:       "",
		IsAdmin:      false,
		IsCreator:    false,
		AccessKey:    "",
		AccessSecret: "",
		Status:       model.DELETED,
	}
	if lab.Status == model.DELETED {
		return resp, nil
	}

	count, err := l.envStore.Count(ctx, &model.LaboratoryMember{}, map[string]any{
		"lab_id":  lab.ID,
		"user_id": userInfo.ID,
	})
	if err != nil || count == 0 {
		return nil, code.NoPermission
	}

	if lab.Status != model.DELETED {
		resp.UUID = lab.UUID
		resp.Name = lab.Name
		resp.UserID = lab.UserID
		resp.IsAdmin = lab.UserID == userInfo.ID
		resp.IsCreator = lab.UserID == userInfo.ID
		resp.AccessKey = lab.AccessKey
		resp.AccessSecret = lab.AccessSecret
		resp.Status = lab.Status
	}

	return resp, nil
}

func (l *lab) CreateResource(ctx context.Context, req *eo.ResourceReq) error {
	if len(req.Resources) == 0 {
		return code.ResourceIsEmptyErr
	}

	labInfo := auth.GetCurrentUser(ctx)
	if labInfo == nil {
		return code.UnLogin
	}
	labData, err := l.envStore.GetLabByAkSk(ctx, labInfo.AccessKey, labInfo.AccessSecret)
	if err != nil {
		return err
	}

	return db.DB().ExecTx(ctx, func(txCtx context.Context) error {
		resDatas := utils.FilterSlice(req.Resources, func(item *eo.Resource) (*model.ResourceNodeTemplate, bool) {
			data := &model.ResourceNodeTemplate{
				Name:         item.RegName,
				LabID:        labData.ID,     // 实验室的 id
				UserID:       labData.UserID, // 创建实验室的 user id
				Header:       item.RegName,
				Footer:       "",
				Icon:         item.Icon,
				Description:  item.Description,
				Model:        item.Model,
				Module:       item.Class.Module,
				ResourceType: item.ResourceType,
				Language:     item.Class.Type,
				StatusTypes:  item.Class.StatusTypes,
				ConfigInfo:   item.ConfigInfo,
				Tags:         item.Tags,
				DataSchema: utils.SafeValue(func() datatypes.JSON {
					return item.InitParamSchema.Data.Properties
				}, datatypes.JSON{}),
				ConfigSchema: utils.SafeValue(
					func() datatypes.JSON { return item.InitParamSchema.Config.Properties },
					datatypes.JSON{}),
			}
			item.SelfDB = data
			return data, true
		})

		if err := l.envStore.UpsertResourceNodeTemplate(txCtx, resDatas); err != nil {
			return err
		}

		// if err := l.createConfigInfo(txCtx, req.Resources); err != nil {
		// 	return err
		// }

		if err := l.createHandle(txCtx, req.Resources); err != nil {
			return err
		}

		actions, err := l.createWorkflowNodeTemplate(txCtx, req.Resources)
		if err != nil {
			return err
		}

		return l.createActionHandles(txCtx, actions)
	})
}

func (l *lab) createWorkflowNodeTemplate(ctx context.Context, res []*eo.Resource) ([]*model.WorkflowNodeTemplate, error) {
	resDeviceAction, err := utils.FilterSliceWithErr(res, func(item *eo.Resource) ([]*model.WorkflowNodeTemplate, bool, error) {
		actions := make([]*model.WorkflowNodeTemplate, 0, len(item.Class.ActionValueMappings))
		for actionName, action := range item.Class.ActionValueMappings {
			if actionName == "" {
				return nil, false, code.RegActionNameEmptyErr
			}

			actions = append(actions, &model.WorkflowNodeTemplate{
				LabID:          item.SelfDB.LabID,
				ResourceNodeID: item.SelfDB.ID,
				Name:           actionName,
				DisplayName:    utils.Or(action.DisplayName, actionName),
				Goal:           action.Goal,
				GoalDefault:    action.GoalDefault,
				Feedback:       action.Feedback,
				Result:         action.Result,
				Schema:         action.Schema,
				Type:           action.Type,
				Handles:        action.Handles,
				Header:         actionName,
				Footer:         item.SelfDB.Name,
			})
		}
		return actions, true, nil
	})
	if err != nil {
		return nil, err
	}
	return resDeviceAction, l.envStore.UpsertWorkflowNodeTemplate(ctx, resDeviceAction)
}

func (l *lab) createHandle(ctx context.Context, res []*eo.Resource) error {
	resDeviceHandles, err := utils.FilterSliceWithErr(res, func(item *eo.Resource) ([]*model.ResourceHandleTemplate, bool, error) {
		handles := make([]*model.ResourceHandleTemplate, 0, len(item.Handles))
		for _, handle := range item.Handles {
			handles = append(handles, &model.ResourceHandleTemplate{
				ResourceNodeID: item.SelfDB.ID,
				Name:           handle.HandlerKey,
				DisplayName:    handle.Label,
				Type:           handle.DataType,
				IOType:         handle.IoType,
				Source:         handle.DataSource,
				Key:            handle.DataKey,
				Side:           handle.Side,
			})
		}
		return handles, true, nil
	})
	if err != nil {
		return err
	}

	return l.envStore.UpsertResourceHandleTemplate(ctx, resDeviceHandles)
}

// func (l *lab) createConfigInfo(ctx context.Context, res []*environment.Resource) error {
// 	_, err := utils.FilterSliceWithErr(res, func(item *environment.Resource) ([]*model.ResourceNodeTemplate, bool, error) {
// 		res, err1 := utils.FilterSliceWithErr(item.ConfigInfo, func(conf *environment.Config) ([]*model.ResourceNodeTemplate, bool, error) {
// 			innerConfig := &environment.InnerBaseConfig{}
// 			if err := json.Unmarshal(conf.Config, innerConfig); err != nil {
// 				logger.Errorf(ctx, "CreateResource Unmarshal innerbaseconfig fail err: %+v", err)
// 				return nil, false, err
// 			}
//
// 			pose := model.Pose{
// 				Layout:   "2d",
// 				Position: conf.Position,
// 				Size: model.Size{
// 					Width:  int(innerConfig.SizeX),
// 					Height: int(innerConfig.SizeY),
// 					Depth:  int(innerConfig.SizeZ),
// 				},
// 				Scale: model.Scale{},
// 				Rotation: model.Rotation{
// 					X: innerConfig.Rotation.X,
// 					Y: innerConfig.Rotation.Y,
// 					Z: innerConfig.Rotation.Z,
// 				},
// 			}
//
// 			data := &model.ResourceNodeTemplate{
// 				Name:         conf.ID,
// 				ParentID:     utils.Ternary(conf.Parent == "", item.SelfDB.ID, 0),
// 				LabID:        item.SelfDB.LabID,
// 				UserID:       item.SelfDB.UserID,
// 				Header:       conf.Name,
// 				Footer:       "",
// 				Version:      utils.Or(item.Version, "0.0.1"),
// 				Icon:         "",
// 				Description:  nil,
// 				Model:        datatypes.JSON{},
// 				Module:       "",
// 				ResourceType: conf.Type,
// 				Language:     "",
// 				StatusTypes:  datatypes.JSON{},
// 				Tags:         datatypes.JSONSlice[string]{},
// 				DataSchema:   conf.Data,
// 				ConfigSchema: conf.Config,
// 				Pose:         datatypes.NewJSONType(pose),
//
// 				ParentNode: item.SelfDB,
// 				ParentName: conf.Parent,
// 			}
// 			return []*model.ResourceNodeTemplate{data}, true, nil
// 		})
//
// 		if err1 != nil {
// 			logger.Errorf(ctx, "createConfigInfo err: %+v", err1)
// 			return nil, false, err1
// 		}
//
// 		preBuildNodes := utils.FilterSlice(res, func(item *model.ResourceNodeTemplate) (*utils.Node[string, *model.ResourceNodeTemplate], bool) {
// 			return &utils.Node[string, *model.ResourceNodeTemplate]{
// 				Name:   item.Name,
// 				Parent: item.ParentName,
// 				Data:   item,
// 			}, true
// 		})
//
// 		buildNodes, err := utils.BuildHierarchy(preBuildNodes)
// 		if err != nil {
// 			return nil, false, err
// 		}
//
// 		// FIXME: 是否还有优化空间
// 		upsertNodeMap := make(map[string]*model.ResourceNodeTemplate)
// 		for _, datas := range buildNodes {
// 			for _, data := range datas {
// 				if data.ParentName != "" {
// 					parentNode, ok := upsertNodeMap[data.ParentName]
// 					if ok {
// 						data.ParentID = parentNode.ID
// 					} else {
// 						logger.Errorf(ctx, "can not found config info parent config: %+v", data)
// 						return nil, false, code.ParamErr.WithMsg(fmt.Sprintf("can not found config info parent config: %+v", data))
// 					}
// 				}
// 			}
//
// 			if err := l.envStore.UpsertResourceNodeTemplate(ctx, datas); err != nil {
// 				return nil, false, err
// 			}
//
// 			for _, data := range datas {
// 				upsertNodeMap[data.Name] = data
// 			}
// 		}
//
// 		return res, true, err
// 	})
//
// 	return err
// }

func (l *lab) createActionHandles(ctx context.Context, actions []*model.WorkflowNodeTemplate) error {
	resHandles, _ := utils.FilterSliceWithErr(actions, func(item *model.WorkflowNodeTemplate) ([]*model.WorkflowHandleTemplate, bool, error) {
		resHi, _ := utils.FilterSliceWithErr(item.Handles.Data().Input, func(h *model.Handle) ([]*model.WorkflowHandleTemplate, bool, error) {
			return []*model.WorkflowHandleTemplate{{
				WorkflowNodeID: item.ID,
				HandleKey:      h.HandlerKey,
				IoType:         "target",
				DisplayName:    h.Label,
				Type:           h.DataType,
				DataSource:     h.DataSource,
				DataKey:        h.DataKey,
			}}, true, nil
		})
		resHo, _ := utils.FilterSliceWithErr(item.Handles.Data().Output, func(h *model.Handle) ([]*model.WorkflowHandleTemplate, bool, error) {
			return []*model.WorkflowHandleTemplate{{
				WorkflowNodeID: item.ID,
				HandleKey:      h.HandlerKey,
				IoType:         "source",
				DisplayName:    h.Label,
				Type:           h.DataType,
				DataSource:     h.DataSource,
				DataKey:        h.DataKey,
			}}, true, nil
		})

		resH := make([]*model.WorkflowHandleTemplate, 0, len(resHi)+len(resHo)+2)

		resH = append(resH, &model.WorkflowHandleTemplate{
			WorkflowNodeID: item.ID,
			HandleKey:      "ready",
			IoType:         "target",
		})
		resH = append(resH, &model.WorkflowHandleTemplate{
			WorkflowNodeID: item.ID,
			HandleKey:      "ready",
			IoType:         "source",
		})
		resH = append(resH, resHi...)
		resH = append(resH, resHo...)

		return resH, true, nil
	})

	return l.envStore.UpsertActionHandleTemplate(ctx, resHandles)
}

func (l *lab) LabList(ctx context.Context, req *common.PageReq) (*common.PageMoreResp[[]*eo.LaboratoryResp], error) {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return nil, code.UnLogin
	}

	labs, err := l.envStore.GetLabByUserID(ctx, &common.PageReqT[string]{
		PageReq: *req,
		Data:    userInfo.ID,
	})
	if err != nil {
		return nil, err
	}

	labIDs := utils.FilterSlice(labs.Data, func(labMember *model.LaboratoryMember) (int64, bool) {
		return labMember.LabID, true
	})

	labDatas := make([]*model.Laboratory, 0, len(labIDs))
	if err := l.envStore.FindDatas(ctx, &labDatas, map[string]any{
		"id": labIDs,
	}); err != nil {
		return nil, err
	}

	labMap := utils.Slice2Map(labDatas, func(l *model.Laboratory) (int64, *model.Laboratory) {
		return l.ID, l
	})

	labMemberMap := l.envStore.GetLabMemberCount(ctx, labIDs...)

	labResp := utils.FilterSlice(labs.Data, func(item *model.LaboratoryMember) (*eo.LaboratoryResp, bool) {
		lab, ok := labMap[item.LabID]
		if !ok {
			logger.Infof(ctx, "can not found lab id: %+d", item.LabID)
			return nil, false
		}

		return &eo.LaboratoryResp{
			UUID:        lab.UUID,
			Name:        lab.Name,
			UserID:      lab.UserID,
			Description: lab.Description,
			MemberCount: labMemberMap[lab.ID],
			IsAdmin:     lab.UserID == userInfo.ID,
			IsCreator:   lab.UserID == userInfo.ID,
			IsPin:       item.PinTime != nil,
		}, true
	})

	return &common.PageMoreResp[[]*eo.LaboratoryResp]{
		Data:     labResp,
		Page:     labs.Page,
		HasMore:  labs.Total > int64(labs.Page+1)*int64(labs.PageSize),
		PageSize: labs.PageSize,
	}, nil
}

func (l *lab) LabMemberList(ctx context.Context, req *eo.LabMemberReq) (*common.PageResp[[]*eo.LabMemberResp], error) {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return nil, code.UnLogin
	}

	lab, err := l.envStore.GetLabByUUID(ctx, req.LabUUID)
	if err != nil {
		return nil, code.CanNotGetLabIDErr
	}

	req.Normalize()
	c, err := l.envStore.Count(ctx, &model.LaboratoryMember{}, map[string]any{
		"user_id": userInfo.ID,
		"lab_id":  lab.ID,
	})
	if err != nil {
		return nil, err
	}
	if c == 0 {
		return nil, code.NoPermission
	}

	labMembers, err := l.envStore.GetLabByLabID(ctx, &common.PageReqT[int64]{
		PageReq: req.PageReq,
		Data:    lab.ID,
	})
	if err != nil {
		return nil, err
	}

	allUserIDs := utils.FilterUniqSlice(labMembers.Data, func(l *model.LaboratoryMember) (string, bool) {
		return l.UserID, true
	})

	userDatas, err := l.accountClient.BatchGetUserInfo(ctx, allUserIDs)
	if err != nil {
		return nil, err
	}

	userInfoMap := utils.Slice2Map(userDatas, func(userInfo *model.UserData) (string, *model.UserData) {
		return userInfo.ID, userInfo
	})

	userRole := make([]*model.UserRole, 0, req.PageSize)
	if err := l.envStore.FindDatas(ctx, &userRole, map[string]any{
		"lab_id":  lab.ID,
		"user_id": allUserIDs,
	}); err != nil {
		return nil, err
	}

	userRolesMap := utils.Slice2MapSlice(userRole, func(u *model.UserRole) (string, int64, bool) {
		return u.UserID, u.CustomRoleID, true
	})

	roleIDs := utils.FilterUniqSlice(userRole, func(r *model.UserRole) (int64, bool) {
		return r.CustomRoleID, true
	})

	roles := make([]*model.CustomRole, 0, len(roleIDs))
	if err := l.envStore.FindDatas(ctx, &roles, map[string]any{
		"id": roleIDs,
	}); err != nil {
		return nil, err
	}

	roleMap := utils.Slice2Map(roles, func(r *model.CustomRole) (int64, *model.CustomRole) {
		return r.ID, r
	})

	resp := utils.FilterSlice(labMembers.Data, func(l *model.LaboratoryMember) (*eo.LabMemberResp, bool) {
		userInfo, ok := userInfoMap[l.UserID]
		if !ok {
			logger.Errorf(ctx, "can not get user info user id: %s", l.UserID)
			return nil, false
		}
		return &eo.LabMemberResp{
			UUID:   l.UUID,
			UserID: l.UserID,
			LabID:  l.LabID,
			Role:   l.Role,
			Roles: utils.FilterSlice(userRolesMap[l.UserID], func(roleID int64) (*eo.RoleInfo, bool) {
				role, ok := roleMap[roleID]
				return &eo.RoleInfo{
					RoleUUID: role.UUID,
					RoleName: role.RoleName,
				}, ok
			}),
			DisplayName: userInfo.DisplayName,
			Email:       userInfo.Email,
			Phone:       userInfo.Phone,
			Name:        userInfo.Name,
			IsAdmin:     lab.UserID == userInfo.ID,
			IsCreator:   lab.UserID == userInfo.ID,
		}, true
	})

	return &common.PageResp[[]*eo.LabMemberResp]{
		Total:    labMembers.Total,
		Page:     labMembers.Page,
		PageSize: labMembers.PageSize,
		Data:     resp,
	}, nil
}

func (l *lab) DelLabMember(ctx context.Context, req *eo.DelLabMemberReq) error {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return code.UnLogin
	}

	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID <= 0 {
		return code.LabNotFound
	}

	// 只有管理员可以删除
	if count, err := l.envStore.Count(ctx, &model.LaboratoryMember{}, map[string]any{
		"lab_id":  labID,
		"user_id": userInfo.ID,
		"role":    common.Admin,
	}); err != nil {
		return err
	} else if count == 0 {
		return code.NoPermission
	}

	data := &model.LaboratoryMember{}
	if err := l.envStore.GetData(ctx, data, map[string]any{
		"uuid": req.MemberUUID,
	}); err != nil {
		return err
	}

	if err := l.envStore.DelData(ctx, &model.UserRole{}, map[string]any{
		"lab_id":  labID,
		"user_id": data.UserID,
	}); err != nil {
		return err
	}

	return l.envStore.DelData(ctx, &model.LaboratoryMember{}, map[string]any{
		"uuid": req.MemberUUID,
	})
}

func (l *lab) CreateInvite(ctx context.Context, req *eo.InviteReq) (*eo.InviteResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return nil, code.UnLogin
	}

	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID <= 0 {
		return nil, code.LabNotFound
	}

	// 只有管理员可以创建
	if count, err := l.envStore.Count(ctx, &model.LaboratoryMember{}, map[string]any{
		"lab_id":  labID,
		"user_id": userInfo.ID,
		"role":    common.Admin,
	}); err != nil {
		return nil, err
	} else if count == 0 {
		return nil, code.NoPermission
	}

	roles := make([]*model.CustomRole, 0, 1)
	condition := map[string]any{
		"lab_id": labID,
	}
	if len(req.RoleUUIDs) == 0 {
		condition["role_name"] = common.Normal
	} else {
		condition["uuid"] = req.RoleUUIDs
	}

	if err := l.envStore.FindDatas(ctx, &roles, condition, "id"); err != nil {
		return nil, err
	}

	if len(roles) == 0 {
		logger.Warnf(ctx, "CreateInvite.get roles role uuid list: %+v", req.RoleUUIDs)
		return nil, code.RoleNotExistErr
	}

	data := &model.LaboratoryInvitation{
		ExpiresAt: time.Now().Add(7 * 24 * time.Hour),
		Type:      model.InvitationTypeLab,
		ThirdID:   strconv.FormatInt(labID, 10),
		UserID:    userInfo.ID,
		RoleIDs: datatypes.NewJSONSlice(utils.FilterSlice(roles, func(r *model.CustomRole) (int64, bool) {
			return r.ID, true
		})),
	}

	if err := l.inviteStore.CreateData(ctx, data); err != nil {
		return nil, err
	}

	return &eo.InviteResp{
		Path: fmt.Sprintf("/api/v1/lab/invite/%s", data.UUID),
	}, nil
}

func (l *lab) AcceptInvite(ctx context.Context, req *eo.AcceptInviteReq) (*eo.AcceptInviteResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return nil, code.UnLogin
	}

	invitation := &model.LaboratoryInvitation{}
	if err := l.envStore.GetData(ctx, invitation, map[string]any{
		"uuid": req.UUID,
	}); err != nil {
		logger.Warnf(ctx, "AcceptInvite err: %+v", err)
		return nil, code.LabInviteNotFoundErr
	}

	if invitation.ExpiresAt.Unix() < time.Now().Unix() {
		return nil, code.InviteExpiredErr
	}

	// 管理员进入自己的实验室
	if invitation.UserID == userInfo.ID {
		return l.addLabMember(ctx, invitation)
	}

	switch invitation.Type {
	case model.InvitationTypeLab:
		return l.addLabMember(ctx, invitation)

	default:
		logger.Warnf(ctx, "can not found this invite type: %+s", invitation.Type)
	}

	return nil, code.LabInviteNotFoundErr
}

func (l *lab) addLabMember(ctx context.Context, data *model.LaboratoryInvitation) (*eo.AcceptInviteResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	labID, err := strconv.ParseInt(data.ThirdID, 10, 64)
	if err != nil {
		return nil, code.InvalidateThirdID.WithErr(err)
	}

	// 获取实验室
	labInfo := &model.Laboratory{}
	if err := l.envStore.GetData(ctx, labInfo, map[string]any{
		"id": labID,
	}, "uuid", "name"); err != nil {
		return nil, err
	}

	if userInfo.ID == data.UserID {
		return &eo.AcceptInviteResp{
			LabUUID: labInfo.UUID,
			Name:    labInfo.Name,
		}, nil
	}
	if err := l.envStore.AddLabMember(ctx, &model.LaboratoryMember{
		UserID: userInfo.ID,
		LabID:  labID,
		Role:   common.Normal,
	}); err != nil {
		return nil, err
	}

	roles := make([]*model.CustomRole, 0, 1)
	condition := map[string]any{
		"lab_id": labID,
	}
	if len(data.RoleIDs) == 0 {
		condition["role_name"] = common.Normal
	} else {
		condition["id"] = []int64(data.RoleIDs)
	}

	if err := l.envStore.FindDatas(ctx, &roles, condition, "id"); err != nil {
		return nil, err
	}

	if len(roles) == 0 {
		return nil, code.RoleNotExistErr
	}

	roleIDs := utils.FilterSlice(roles, func(r *model.CustomRole) (int64, bool) {
		return r.ID, true
	})

	if err := l.envStore.AddUserRole(ctx, labID, roleIDs, userInfo.ID); err != nil {
		return nil, err
	}

	return &eo.AcceptInviteResp{
		LabUUID: labInfo.UUID,
		Name:    labInfo.Name,
	}, nil
}

func (l *lab) UserInfo(ctx context.Context) (*model.UserData, error) {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return nil, code.UnLogin
	}

	return l.accountClient.GetUserInfo(ctx, userInfo.ID)
}

func (l *lab) PinLab(ctx context.Context, req *eo.PinLabReq) error {
	userInfo := auth.GetCurrentUser(ctx)
	// 判断该用户是否是该实验室成员
	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID <= 0 {
		return code.LabNotFound
	}

	if count, err := l.envStore.Count(ctx, &model.LaboratoryMember{}, map[string]any{
		"lab_id":  labID,
		"user_id": userInfo.ID,
	}); err != nil || count == 0 {
		return code.NoPermission
	}

	// 修改置顶时间
	member := &model.LaboratoryMember{
		PinTime: nil,
	}

	if req.PinLab {
		now := time.Now()
		member.PinTime = &now
	}

	if err := l.envStore.UpdateData(ctx, member, map[string]any{
		"user_id": userInfo.ID,
		"lab_id":  labID,
	}, "pin_time"); err != nil {
		return err
	}

	return nil
}

func (l *lab) Policy(ctx context.Context, req *eo.PolicyReq) (*repo.ResourcePerm, error) {
	userData := auth.GetCurrentUser(ctx)
	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		return nil, code.LabNotFound
	}

	labMember := &model.LaboratoryMember{}
	if err := l.envStore.GetData(ctx, labMember, map[string]any{
		"lab_id":  labID,
		"user_id": userData.ID,
	}); err != nil {
		return nil, err
	}

	userRoles := make([]*model.UserRole, 0, 1)
	if err := l.envStore.FindDatas(ctx, &userRoles, map[string]any{
		"lab_id":  labID,
		"user_id": userData.ID,
	}, "custom_role_id"); err != nil {
		return nil, err
	}

	customRoleIDs := utils.FilterSlice(userRoles, func(u *model.UserRole) (int64, bool) {
		return u.CustomRoleID, true
	})

	customRoles := make([]*model.CustomRole, 0, 1)
	if err := l.envStore.FindDatas(ctx, &customRoles, map[string]any{
		"id": customRoleIDs,
	}, "role_name"); err != nil {
		return nil, err
	}

	if len(customRoles) == 0 {
		return &repo.ResourcePerm{Permissions: make(map[string][]common.Perm)}, nil
	}
	roles := utils.FilterSlice(customRoles, func(c *model.CustomRole) (string, bool) {
		return c.RoleName, true
	})

	res, err := l.policy.GetRolePerms(ctx, &repo.UserPermRes{
		LabID:  labID,
		UserID: userData.ID,
		Roles:  roles,
	})
	if err != nil {
		return nil, err
	}

	return res, nil
}

func (l *lab) isLabAdmin(ctx context.Context, userID string, labID int64) error {
	labRole := &model.CustomRole{}
	if err := l.envStore.GetData(ctx, labRole, map[string]any{
		"lab_id":    labID,
		"role_name": common.Admin,
	}); err != nil {
		return code.LabInnerAdminRoleNoteExist
	}

	userRole := &model.UserRole{}
	if err := l.envStore.GetData(ctx, userRole, map[string]any{
		"lab_id":         labID,
		"user_id":        userID,
		"custom_role_id": labRole.ID,
	}); err != nil {
		return code.NoPermission
	}

	return nil
}

func (l *lab) CreateRole(ctx context.Context, req *eo.CreateRoleReq) (*eo.CreateRoleResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	if slices.Contains([]string{
		string(common.Admin),
		string(common.Normal),
	},
		req.RoleName) {
		return nil, code.LabInnerRoleExist
	}

	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		return nil, code.LabNotFound
	}

	// 根据 user id 该成员是否有权限创建角色，暂时只允许实验室创建者创建角色
	if err := l.isLabAdmin(ctx, userInfo.ID, labID); err != nil {
		return nil, err
	}

	data := &model.CustomRole{
		LabID:       labID,
		RoleName:    req.RoleName,
		Description: req.Description,
	}
	if err := l.envStore.CreateData(ctx, data); err != nil {
		return nil, err
	}

	return &eo.CreateRoleResp{
		RoleUUID: data.UUID,
		RoleName: data.RoleName,
	}, nil
}

func (l *lab) RoleList(ctx context.Context, req *eo.RoleListReq) (*eo.RoleListResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	_, err := l.envStore.GetRole(ctx, req.LabUUID, userInfo.ID)
	if err != nil {
		return nil, err
	}

	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		return nil, code.LabNotFound
	}

	labRoles := make([]*model.CustomRole, 0, 1)
	if err := l.envStore.FindDatas(ctx, &labRoles, map[string]any{
		"lab_id": labID,
	}); err != nil {
		return nil, err
	}

	return &eo.RoleListResp{
		Roles: utils.FilterSlice(labRoles, func(r *model.CustomRole) (*eo.Role, bool) {
			return &eo.Role{
				RoleUUID: r.UUID,
				RoleName: r.RoleName,
			}, true
		}),
	}, nil
}

func (l *lab) DelRole(ctx context.Context, req *eo.DelRoleReq) error {
	userInfo := auth.GetCurrentUser(ctx)
	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		return code.LabNotFound
	}

	if err := l.isLabAdmin(ctx, userInfo.ID, labID); err != nil {
		return err
	}

	innerRoles := make([]*model.CustomRole, 0, 2)
	if err := l.envStore.FindDatas(ctx, &innerRoles, map[string]any{
		"lab_id":    labID,
		"role_name": []common.Role{common.Admin, common.Normal},
	}, "uuid"); err != nil {
		return err
	}

	innerUUIDs := utils.FilterSlice(innerRoles, func(c *model.CustomRole) (uuid.UUID, bool) {
		return c.UUID, true
	})

	if slices.Contains(innerUUIDs, req.RoleUUID) {
		return code.NoPermission
	}

	roleID := l.envStore.UUID2ID(ctx, &model.CustomRole{}, req.RoleUUID)[req.RoleUUID]
	if roleID == 0 {
		return code.RoleNotExistErr
	}

	if err := l.envStore.ExecTx(ctx, func(txCtx context.Context) error {
		if err := l.envStore.DelData(txCtx, &model.UserRole{}, map[string]any{
			"lab_id":         labID,
			"custom_role_id": roleID,
		}); err != nil {
			return err
		}

		if err := l.envStore.DelData(txCtx, &model.CustomRole{}, map[string]any{
			"uuid":   req.RoleUUID,
			"lab_id": labID,
		}); err != nil {
			return err
		}
		return nil
	}); err != nil {
		return err
	}

	return nil
}

// 增加角色权限
func (l *lab) ModifyRolePerm(ctx context.Context, req *eo.AddRolePermReq) error {
	if len(req.AddItems) == 0 &&
		len(req.DelPermUUID) == 0 &&
		req.Description == nil &&
		req.Name == nil {
		return nil
	}

	userInfo := auth.GetCurrentUser(ctx)
	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		return code.LabNotFound
	}

	if err := l.isLabAdmin(ctx, userInfo.ID, labID); err != nil {
		return err
	}

	labRole := &model.CustomRole{}
	if err := l.envStore.GetData(ctx, labRole, map[string]any{
		"uuid":   req.RoleUUID,
		"lab_id": labID,
	}); err != nil {
		return err
	}

	keys := make([]string, 0, 2)
	if req.Name != nil && *req.Name != "" {
		labRole.RoleName = *req.Name
		keys = append(keys, "role_name")
	}

	if req.Description != nil {
		labRole.Description = *req.Description
		keys = append(keys, "description")
	}

	permErr := false
	addResUUIDs := utils.FilterUniqSlice(req.AddItems, func(perm *eo.ResPerm) (uuid.UUID, bool) {
		if !slices.Contains([]common.Perm{common.Visible, common.Clickable}, perm.Perm) {
			permErr = true
		}

		return perm.ResourceUUID, !permErr
	})

	if permErr {
		return code.UnknownPermErr
	}

	policyResData := make([]*model.PolicyResource, 0, len(addResUUIDs))
	if err := l.envStore.FindDatas(ctx, &policyResData, map[string]any{
		"uuid": addResUUIDs,
	}); err != nil {
		return err
	}

	if len(addResUUIDs) != len(policyResData) {
		return code.PolicyResourceNotFoundErr
	}

	resMap := utils.Slice2Map(policyResData, func(p *model.PolicyResource) (uuid.UUID, *model.PolicyResource) {
		return p.UUID, p
	})

	rolePerms := utils.FilterSlice(req.AddItems, func(perm *eo.ResPerm) (*model.CustomRolePerm, bool) {
		return &model.CustomRolePerm{
			CustomRoleID:     labRole.ID,
			PolicyResourceID: resMap[perm.ResourceUUID].ID,
			Perm:             perm.Perm,
		}, true
	})

	return l.envStore.ExecTx(ctx, func(txCtx context.Context) error {
		if len(keys) > 0 {
			err := l.envStore.UpdateData(txCtx, labRole, map[string]any{
				"id": labRole.ID,
			}, keys...)

			if err != nil && (strings.Contains(err.Error(), "Duplicate entry") ||
				strings.Contains(err.Error(), "UNIQUE constraint") ||
				strings.Contains(err.Error(), "duplicate key")) {
				return code.DataExistErr
			}

			if err != nil {
				return err
			}
		}

		if len(rolePerms) > 0 {
			if err := l.envStore.DBWithContext(txCtx).Create(&rolePerms).Error; err != nil {
				logger.Errorf(txCtx, "ModifyRolePerm.create role perm err: %+v", err)
				return code.CreateDataErr.WithErr(err)
			}
		}

		if len(req.DelPermUUID) > 0 {
			if err := l.envStore.DelData(txCtx, &model.CustomRolePerm{}, map[string]any{
				"uuid": req.DelPermUUID,
			}); err != nil {
				return err
			}
		}

		return nil
	})
}

// 角色权限列表, 根据实验室 uuid 和 role uuid 获取该角色拥有的所有权限
func (l *lab) RolePermList(ctx context.Context, req *eo.RolePermListReq) (*eo.RolePermListResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	_ = userInfo
	// TODO: 这块需要权限校验，是个 bug ，稍后修改
	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		return nil, code.LabNotFound
	}

	role := &model.CustomRole{}
	if err := l.envStore.GetData(ctx, role,
		map[string]any{
			"lab_id": labID,
			"uuid":   req.RoleUUID,
		}); err != nil {
		return nil, code.RoleNotExistErr
	}

	resourcePerms := make([]*model.CustomRolePerm, 0, 1)
	if err := l.envStore.FindDatas(ctx, &resourcePerms, map[string]any{
		"custom_role_id": role.ID,
	}); err != nil {
		return nil, err
	}

	resourceIDs := utils.FilterUniqSlice(resourcePerms, func(r *model.CustomRolePerm) (int64, bool) {
		return r.PolicyResourceID, true
	})

	resources := make([]*model.PolicyResource, 0, len(resourceIDs))
	if err := l.envStore.FindDatas(ctx, &resources, map[string]any{
		"id": resourceIDs,
	}); err != nil {
		return nil, err
	}

	resMap := utils.Slice2Map(resources, func(r *model.PolicyResource) (int64, *model.PolicyResource) {
		return r.ID, r
	})

	return &eo.RolePermListResp{
		UUID:        req.RoleUUID,
		Name:        role.RoleName,
		Description: role.Description,
		ResourcePerm: utils.FilterSlice(resourcePerms, func(r *model.CustomRolePerm) (*eo.ResourcePerm, bool) {
			res, ok := resMap[r.PolicyResourceID]
			if !ok {
				return nil, ok
			}

			return &eo.ResourcePerm{
				ResrouceUUID: res.UUID,
				ResourceName: res.Name,
				PermUUID:     r.UUID,
				Perm:         r.Perm,
			}, true
		}),
	}, nil
}

// 删除角色权限
func (l *lab) DelRolePerm(ctx context.Context, req *eo.DelRolePermReq) error {
	userInfo := auth.GetCurrentUser(ctx)
	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		return code.LabNotFound
	}

	if err := l.isLabAdmin(ctx, userInfo.ID, labID); err != nil {
		return err
	}

	if err := l.envStore.DelData(ctx, &model.CustomRolePerm{}, map[string]any{
		"uuid": req.RolePermUUID,
	}); err != nil {
		return err
	}

	return nil
}

// 增加用户角色
func (l *lab) ModifyUserRole(ctx context.Context, req *eo.AddUserRoleReq) (*eo.AddUserRoleResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	if len(req.AddRoles) == 0 &&
		len(req.DelRoles) == 0 {
		return nil, code.ParamErr
	}

	// if userInfo.ID == req.UserID {
	// 	return nil, code.NoPermission
	// }

	// 检查
	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		return nil, code.LabNotFound
	}

	if count, err := l.envStore.Count(ctx, &model.LaboratoryMember{}, map[string]any{
		"lab_id":  labID,
		"user_id": req.UserID,
	}); err != nil || count == 0 {
		return nil, code.NoPermission
	}

	if err := l.isLabAdmin(ctx, userInfo.ID, labID); err != nil {
		return nil, err
	}

	req.AddRoles = utils.RemoveDuplicates(req.AddRoles)
	req.DelRoles = utils.RemoveDuplicates(req.DelRoles)

	roles := utils.RemoveDuplicates(append(req.AddRoles, req.DelRoles...))

	roleMap := l.envStore.UUID2ID(ctx, &model.CustomRole{}, roles...)

	if len(roleMap) != len(roles) {
		return nil, code.RoleNotExistErr
	}

	addRoleIDs := utils.FilterSlice(req.AddRoles, func(u uuid.UUID) (int64, bool) {
		id, ok := roleMap[u]
		return id, ok
	})

	delRoleIDs := utils.FilterSlice(req.DelRoles, func(u uuid.UUID) (int64, bool) {
		id, ok := roleMap[u]
		return id, ok
	})

	addUserRoles := utils.FilterSlice(addRoleIDs, func(id int64) (*model.UserRole, bool) {
		return &model.UserRole{
			LabID:        labID,
			UserID:       req.UserID,
			CustomRoleID: id,
		}, true
	})

	if err := l.envStore.ExecTx(ctx, func(txCtx context.Context) error {
		if len(delRoleIDs) > 0 {
			if err := l.envStore.DelData(txCtx, &model.UserRole{}, map[string]any{
				"lab_id":         labID,
				"user_id":        req.UserID,
				"custom_role_id": delRoleIDs,
			}); err != nil {
				return err
			}
		}

		if len(addRoleIDs) > 0 {
			if err := l.envStore.DBWithContext(txCtx).Create(addUserRoles).Error; err != nil {
				logger.Errorf(ctx, "AddUserRole create user role fail err: %+v", err)
				return code.CreateDataErr
			}
		}
		return nil
	}); err != nil {
		return nil, err
	}

	userRoles := []*model.UserRole{}
	if err := l.envStore.FindDatas(ctx, &userRoles, map[string]any{
		"lab_id":  labID,
		"user_id": req.UserID,
	}); err != nil {
		return nil, err
	}

	roleIDs := utils.FilterSlice(userRoles, func(u *model.UserRole) (int64, bool) {
		return u.CustomRoleID, true
	})

	roleDatas := []*model.CustomRole{}
	if err := l.envStore.FindDatas(ctx, &roleDatas, map[string]any{
		"id": roleIDs,
	}); err != nil {
		return nil, err
	}

	allRoleMap := utils.Slice2Map(roleDatas, func(c *model.CustomRole) (int64, *model.CustomRole) {
		return c.ID, c
	})

	return &eo.AddUserRoleResp{
		RoleItems: utils.FilterSlice(userRoles, func(u *model.UserRole) (*eo.UserRoleInfo, bool) {
			roleInfo, ok := allRoleMap[u.CustomRoleID]
			if !ok {
				return nil, ok
			}

			return &eo.UserRoleInfo{
				UserRoleUUID: u.UUID,
				RoleUUID:     roleInfo.UUID,
				RoleName:     roleInfo.RoleName,
			}, true
		}),
	}, nil
}

// 删除用户角色
func (l *lab) DelUserRole(ctx context.Context, req *eo.DelUserRoleReq) error {
	userInfo := auth.GetCurrentUser(ctx)
	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]

	if labID == 0 {
		return code.LabNotFound
	}

	if err := l.isLabAdmin(ctx, userInfo.ID, labID); err != nil {
		return err
	}

	if count, err := l.envStore.Count(ctx, &model.LaboratoryMember{}, map[string]any{
		"user_id": req.UserID,
		"lab_id":  labID,
	}); err != nil {
		return err
	} else if count == 0 {
		return code.LabMemberNotFoundErr
	}

	return l.envStore.DelData(ctx, &model.UserRole{}, map[string]any{
		"lab_id": labID,
		"uuid":   req.UUID,
	})
}

func (l *lab) PolicyResource(ctx context.Context) (*eo.ResourceResp, error) {
	res := make([]*model.PolicyResource, 0, 1)
	if err := l.envStore.FindDatas(ctx, &res, nil); err != nil {
		return nil, err
	}

	return &eo.ResourceResp{
		Items: utils.FilterSlice(res, func(r *model.PolicyResource) (*eo.ResourceItem, bool) {
			return &eo.ResourceItem{
				UUID:        r.UUID,
				Name:        r.Name,
				Description: r.Description,
			}, true
		}),
	}, nil
}

func (l *lab) CreateProject(ctx context.Context, req *eo.CreateProjectReq) (*eo.CreateProjectResp, error) {
	userInfo := auth.GetCurrentUser(ctx)

	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		return nil, code.LabNotFound
	}

	if count, err := l.envStore.Count(ctx, &model.LaboratoryMember{}, map[string]any{
		"lab_id":  labID,
		"user_id": userInfo.ID,
		"role":    common.Admin,
	}); err != nil || count == 0 {
		return nil, code.NoPermission
	}

	data := &model.Project{
		LabID:       labID,
		Name:        req.Name,
		Description: req.Description,
	}

	if err := l.envStore.ExecTx(ctx, func(txCtx context.Context) error {
		if err := l.envStore.CreateData(txCtx, data); err != nil {
			return err
		}

		return l.envStore.CreateData(ctx, &model.ProjectMember{
			LabID:     labID,
			ProjectID: data.ID,
			UserID:    userInfo.ID,
		})
	}); err != nil {
		return nil, err
	}

	return &eo.CreateProjectResp{
		UUID: data.UUID,
		Name: req.Name,
	}, nil
}

func (l *lab) ModifyProject(ctx context.Context, req *eo.ModifyProjectReq) (*eo.ModifyProjectResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	role, err := l.envStore.GetRole(ctx, req.LabUUID, userInfo.ID)
	if err != nil {
		return nil, err
	}
	if role.Role != common.Admin {
		return nil, code.NoPermission
	}

	data := &model.Project{}
	keys := make([]string, 0, 2)
	if req.Name != nil {
		data.Name = *req.Name
		keys = append(keys, "name")
	}

	if req.Description != nil {
		data.Description = req.Description
	}

	if len(keys) == 0 {
		return nil, code.ParamErr
	}

	if err := l.envStore.UpdateData(ctx, data, map[string]any{
		"lab_id": role.LabID,
		"uuid":   req.ProjectUUID,
	}, keys...); err != nil {
		return nil, err
	}

	if err := l.envStore.GetData(ctx, data, map[string]any{
		"uuid": req.ProjectUUID,
	}); err != nil {
		return nil, err
	}

	return &eo.ModifyProjectResp{
		UUID:        data.UUID,
		Name:        data.Name,
		Description: *data.Description,
	}, nil
}

func (l *lab) AddProjectUser(ctx context.Context, req *eo.AddUserReq) (*eo.AddUserResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	role, err := l.envStore.GetRole(ctx, req.LabUUD, userInfo.ID)
	if err != nil {
		return nil, err
	}

	if role.Role != common.Admin {
		return nil, code.NoPermission
	}

	_, err = l.envStore.GetRole(ctx, req.LabUUD, req.UserID)
	if err != nil {
		return nil, err
	}

	project := &model.Project{}
	if err := l.envStore.GetData(ctx, project, map[string]any{
		"lab_id": role.LabID,
		"uuid":   req.ProjectUUID,
	}); err != nil {
		return nil, err
	}

	data := &model.ProjectMember{
		LabID:     role.LabID,
		ProjectID: project.ID,
		UserID:    req.UserID,
	}
	if err := l.envStore.CreateData(ctx, data); err != nil {
		return nil, err
	}

	return &eo.AddUserResp{
		UUID: data.UUID,
	}, nil
}

func (l *lab) DelProjectUser(ctx context.Context, req *eo.DelUserReq) error {
	// 删除项目，删除工作流，删除记录本

	return nil
}

func (l *lab) ProjectList(ctx context.Context, req *eo.ProjectListReq) (*eo.ProjectListResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	labID := l.envStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		return nil, code.LabNotFound
	}

	if _, err := l.envStore.GetRole(ctx, req.LabUUID, userInfo.ID); err != nil {
		return nil, err
	}

	projects := make([]*model.Project, 0, 1)
	if err := l.envStore.FindDatas(ctx, &projects, map[string]any{
		"lab_id": labID,
	}); err != nil {
		return nil, err
	}

	return &eo.ProjectListResp{
		Items: utils.FilterSlice(projects, func(p *model.Project) (*eo.ProjectItem, bool) {
			return &eo.ProjectItem{
				UUID:        p.UUID,
				Name:        p.Name,
				Description: p.Description,
			}, true
		}),
	}, nil
}

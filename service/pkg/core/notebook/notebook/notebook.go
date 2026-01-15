package notebook

import (
	// 外部依赖
	"context"
	"encoding/json"
	"maps"
	"sort"
	"time"

	r "github.com/redis/go-redis/v9"
	datatypes "gorm.io/datatypes"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	nbCore "github.com/scienceol/opensdl/service/pkg/core/notebook"
	engine "github.com/scienceol/opensdl/service/pkg/core/schedule/engine"
	l "github.com/scienceol/opensdl/service/pkg/core/schedule/lab"
	auth "github.com/scienceol/opensdl/service/pkg/middleware/auth"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	redis "github.com/scienceol/opensdl/service/pkg/middleware/redis"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	// "github.com/scienceol/studio/service/pkg/repo/bohr"
	casdoor "github.com/scienceol/opensdl/service/pkg/repo/casdoor"
	el "github.com/scienceol/opensdl/service/pkg/repo/environment"
	mStore "github.com/scienceol/opensdl/service/pkg/repo/material"
	model "github.com/scienceol/opensdl/service/pkg/model"
	nbRepo "github.com/scienceol/opensdl/service/pkg/repo/notebook"
	wfl "github.com/scienceol/opensdl/service/pkg/repo/workflow"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

type notebookImpl struct {
	notebookStore repo.NotebookRepo
	labStore      repo.LaboratoryRepo
	accountClient repo.Account
	workflowStore repo.WorkflowRepo
	schemaHelper  *schemaHelper
	materialStore repo.MaterialRepo
	rClient       *r.Client
}

func New(ctx context.Context) nbCore.Service {
	return &notebookImpl{
		notebookStore: nbRepo.New(),
		labStore:      el.New(),
		workflowStore: wfl.New(),
		schemaHelper:  &schemaHelper{},
		rClient:       redis.GetClient(),
		materialStore: mStore.NewMaterialImpl(),
		accountClient: func() repo.Account {
			if config.Global().Auth.AuthSource == config.AuthBohr {
				return bohr.New()
			} else {
				return casdoor.NewCasClient()
			}
		}(),
	}
}

// QueryNotebookList 查询 notebook 列表
func (n *notebookImpl) QueryNotebookList(ctx context.Context, req *nbCore.QueryNotebookReq) (*nbCore.QueryNotebookResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return nil, code.UnLogin
	}

	if req.LabUUID.IsNil() {
		return nil, code.ParamErr.WithMsg("lab uuid is empty")
	}

	labID := n.labStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		return nil, code.LabNotFound
	}

	// 查询 notebook 列表
	pageResp, err := n.notebookStore.GetNotebookList(ctx, labID, req)
	if err != nil {
		logger.Errorf(ctx, "QueryNotebookList err: %+v", err)
		return nil, err
	}

	// 获取所有唯一的用户ID
	userIDs := utils.FilterUniqSlice(pageResp.Data, func(nb *model.Notebook) (string, bool) {
		return nb.UserID, nb.UserID != ""
	})

	// 批量获取用户信息
	userDatas, err := n.accountClient.BatchGetUserInfo(ctx, userIDs)
	if err != nil {
		logger.Errorf(ctx, "QueryNotebookList BatchGetUserInfo err: %+v", err)
	}

	// 构建用户信息映射
	userInfoMap := utils.Slice2Map(userDatas, func(userInfo *model.UserData) (string, *model.UserData) {
		return userInfo.ID, userInfo
	})

	// 转换响应格式
	notebookItems := utils.FilterSlice(pageResp.Data, func(nb *model.Notebook) (*nbCore.NotebookItem, bool) {
		// 检查时间是否为零值，如果是零值则设置为 nil
		var startTime *time.Time
		if !nb.StartTime.IsZero() {
			startTime = &nb.StartTime
		}

		var finishedTime *time.Time
		if !nb.FinishedTime.IsZero() {
			finishedTime = &nb.FinishedTime
		}

		item := &nbCore.NotebookItem{
			UUID:         nb.UUID,
			Name:         nb.Name,
			Status:       nb.Status,
			UserID:       nb.UserID,
			SubmitTime:   nb.SubmitTime,
			StartTime:    startTime,
			FinishedTime: finishedTime,
			CreatedAt:    nb.CreatedAt,
			UpdatedAt:    nb.UpdatedAt,
		}

		// 填充用户信息
		if userInfo, ok := userInfoMap[nb.UserID]; ok {
			item.UserName = utils.Or(userInfo.Name, userInfo.DisplayName)
			item.DisplayName = userInfo.DisplayName
		}

		return item, true
	})

	return &nbCore.QueryNotebookResp{
		PageResp: common.PageResp[[]*nbCore.NotebookItem]{
			Total:    pageResp.Total,
			Page:     pageResp.Page,
			PageSize: pageResp.PageSize,
			Data:     notebookItems,
		},
	}, nil
}

// CreateNotebook 创建 notebook
func (n *notebookImpl) CreateNotebook(ctx context.Context, req *nbCore.CreateNotebookReq) (*nbCore.CreateNotebookResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return nil, code.UnLogin
	}

	// 校验 NodeParams 中的每个 NotebookParam
	if len(req.NodeParams) == 0 {
		return nil, code.ParamErr.WithMsg("node_params cannot be empty")
	}

	// 校验：每个数组的长度必须相同
	firstGroupLength := len(req.NodeParams[0].Datas)
	if firstGroupLength == 0 {
		return nil, code.ParamErr.WithMsg("node_params[0] cannot be empty")
	}

	for groupIndex, groupParams := range req.NodeParams {
		// 校验数组长度是否相同
		if len(groupParams.Datas) != firstGroupLength {
			return nil, code.ParamErr.WithMsgf("node_params[%d] length (%d) must be equal to node_params[0] length (%d)",
				groupIndex, len(groupParams.Datas), firstGroupLength)
		}

		// 校验：同一个数组下的 node_uuid 不能重复
		nodeUUIDSet := make(map[uuid.UUID]bool)
		for paramIndex, param := range groupParams.Datas {
			// 校验 NodeUUID 必填
			if param.NodeUUID.IsNil() {
				return nil, code.ParamErr.WithMsgf("node_params[%d][%d].node_uuid is required", groupIndex, paramIndex)
			}

			// 检查 node_uuid 是否重复
			if nodeUUIDSet[param.NodeUUID] {
				return nil, code.ParamErr.WithMsgf("node_params[%d] contains duplicate node_uuid: %s", groupIndex, param.NodeUUID)
			}
			nodeUUIDSet[param.NodeUUID] = true

			// 校验 Param 必填（检查 JSON 是否为空或 null）
			paramStr := string(param.Param)
			if len(param.Param) == 0 || paramStr == "null" || paramStr == "" {
				return nil, code.ParamErr.WithMsgf("node_params[%d][%d].param is required", groupIndex, paramIndex)
			}
		}
	}

	// 转换 lab_uuid 为 lab_id
	labID := n.labStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		logger.Errorf(ctx, "CreateNotebook lab not exist uuid: %s", req.LabUUID.String())
		return nil, code.LabNotFound
	}

	nodeUUIDs := utils.FilterUniqSlice(req.NodeParams[0].Datas, func(param *nbCore.NotebookParam) (uuid.UUID, bool) {
		return param.NodeUUID, true
	})

	nodeUUID2IDMap := n.notebookStore.UUID2ID(ctx, &model.WorkflowNode{}, nodeUUIDs...)
	if len(nodeUUID2IDMap) != len(nodeUUIDs) {
		return nil, code.ParamErr.WithMsg("node uuid not exist")
	}

	var notebookUUID uuid.UUID
	err := n.notebookStore.ExecTx(ctx, func(txCtx context.Context) error {
		// 转换 workflow_uuid 为 workflow_id
		workflowID := n.notebookStore.UUID2ID(txCtx, &model.Workflow{}, req.WorkflowUUID)[req.WorkflowUUID]
		if workflowID == 0 {
			logger.Errorf(txCtx, "CreateNotebook workflow uuid not found: %s", req.WorkflowUUID)
			return code.RecordNotFound.WithMsgf("workflow uuid: %s not found", req.WorkflowUUID)
		}

		// 创建 Notebook 表记录
		notebook := &model.Notebook{
			LabID:      labID,
			WorkflowID: workflowID,
			UserID:     userInfo.ID,
			Name:       req.Name,
			Status:     model.NotebookStatusInit,
			SubmitTime: time.Now(),
		}

		if err := n.notebookStore.CreateNotebook(txCtx, notebook); err != nil {
			logger.Errorf(txCtx, "CreateNotebook err: %+v", err)
			return err
		}

		notebookUUID = notebook.UUID
		for groupIndex, groupParams := range req.NodeParams {
			// 创建 NotebookGroup
			notebookGroup := &model.NotebookGroup{
				NotebookID:  notebook.ID,
				Status:      model.NotebookStatusInit,
				SampleUUIDs: groupParams.SampleUUIDs,
			}

			if err := n.notebookStore.CreateNotebookGroup(txCtx, notebookGroup); err != nil {
				logger.Errorf(txCtx, "CreateNotebookGroup err: %+v, groupIndex: %d", err, groupIndex)
				return err
			}

			notebookParams := utils.FilterSlice(groupParams.Datas, func(p *nbCore.NotebookParam) (*model.NotebookParam, bool) {
				return &model.NotebookParam{
					NotebookGroupID: notebookGroup.ID,
					WorkflowNodeID:  nodeUUID2IDMap[p.NodeUUID],
					Param:           p.Param,
				}, true
			})

			// 批量创建 NotebookParam
			if err := n.notebookStore.DBWithContext(txCtx).Create(notebookParams).Error; err != nil {
				logger.Errorf(txCtx, "CreateNotebookParam batch create err: %+v, groupIndex: %d", err, groupIndex)
				return code.CreateDataErr.WithMsg(err.Error())
			}
		}

		return nil
	})
	if err != nil {
		return nil, err
	}

	// 运行 notebook
	data := l.ApiData[engine.NotebookInfo]{
		ApiMsg: l.ApiMsg{
			Action: l.StartNotebook,
		},
		Data: engine.NotebookInfo{
			NotebookUUID: notebookUUID,
			UserID:       userInfo.ID,
			LabUUID:      req.LabUUID,
			LabID:        labID,
		},
	}

	dataB, _ := json.Marshal(data)
	logger.Infof(ctx, "runNotebook ============ data: %s", string(dataB))

	ret := n.rClient.LPush(ctx, utils.LabTaskName(req.LabUUID), dataB)
	if ret.Err() != nil {
		logger.Errorf(ctx, "runWorkflow ============ send data error: %+v", ret.Err())
	}

	return &nbCore.CreateNotebookResp{
		UUID: notebookUUID,
	}, nil
}

// DeleteNotebook 删除 notebook（软删除）
func (n *notebookImpl) DeleteNotebook(ctx context.Context, req *nbCore.DeleteNotebookReq) error {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return code.UnLogin
	}

	// 根据 UUID 获取 notebook（GetNotebookByUUID 已经过滤了已删除的记录）
	notebook, err := n.notebookStore.GetNotebookByUUID(ctx, req.UUID)
	if err != nil {
		logger.Errorf(ctx, "DeleteNotebook GetNotebookByUUID err: %+v", err)
		return err
	}

	// 验证权限：确保用户有权限删除该 notebook（只有创建者可以删除）
	if notebook.UserID != userInfo.ID {
		return code.NoPermission
	}

	// 软删除 notebook（会将 status 更新为 deleted，同时软删除关联的 group）
	if err := n.notebookStore.DeleteNotebook(ctx, notebook.ID); err != nil {
		logger.Errorf(ctx, "DeleteNotebook err: %+v", err)
		return err
	}

	return nil
}

func (n *notebookImpl) NotebookSchema(ctx context.Context, req *nbCore.NotebookSchemaReq) (*nbCore.NotebookSchemaResp, error) {
	wk := &model.Workflow{}
	if err := n.workflowStore.GetData(ctx, wk, map[string]any{
		"uuid": req.UUID,
	}, "id", "lab_id"); err != nil || wk.ID <= 0 {
		return nil, code.WorkflowNotExistErr
	}

	nodes, err := n.workflowStore.GetWorkflowNodes(ctx, map[string]any{
		"workflow_id": wk.ID,
		"disabled":    false,
		"type": []model.WorkflowNodeType{
			model.WorkflowNodeILab,
		},
	}, "id", "uuid", "workflow_node_id", "name", "param")
	if err != nil {
		return nil, err
	}
	nodeUUIDs := utils.FilterSlice(nodes, func(n *model.WorkflowNode) (uuid.UUID, bool) {
		return n.UUID, true
	})
	if len(nodeUUIDs) == 0 {
		return nil, code.Success
	}

	edges, err := n.workflowStore.GetWorkflowEdges(ctx, nodeUUIDs)
	if err != nil {
		return nil, err
	}

	nodes, err = SortWorkflowNodesByDAG(nodes, edges)
	if err != nil {
		return nil, err
	}

	workflowTplIDs := utils.FilterUniqSlice(nodes, func(n *model.WorkflowNode) (int64, bool) {
		return n.WorkflowNodeID, n.WorkflowNodeID > 0
	})

	if len(workflowTplIDs) == 0 {
		return nil, code.Success
	}

	tplNodes := make([]*model.WorkflowNodeTemplate, 0, len(workflowTplIDs))
	if err := n.workflowStore.FindDatas(ctx, &tplNodes,
		map[string]any{"id": workflowTplIDs},
		"id", "schema"); err != nil {
		return nil, err
	}

	if len(tplNodes) == 0 {
		return nil, code.Success
	}

	tplNodes = utils.FilterSlice(tplNodes, func(t *model.WorkflowNodeTemplate) (*model.WorkflowNodeTemplate, bool) {
		return t, true
	})
	if len(tplNodes) == 0 {
		return nil, code.Success
	}

	tplMap := utils.Slice2Map(tplNodes, func(t *model.WorkflowNodeTemplate) (
		int64, *model.WorkflowNodeTemplate,
	) {
		return t.ID, t
	})

	materialNodes := make([]*model.MaterialNode, 0, 1)
	if err := n.materialStore.FindDatas(ctx, &materialNodes, map[string]any{
		"lab_id": wk.LabID,
	}, "id", "uuid", "name", "display_name", "type", "parent_id"); err != nil {
		return nil, err
	}

	resourceNodeTemplates := make([]*model.ResourceNodeTemplate, 0, 1)
	if err := n.materialStore.FindDatas(ctx, &resourceNodeTemplates, map[string]any{
		"lab_id":        wk.LabID,
		"resource_type": "resource",
	}, "id", "uuid", "name"); err != nil {
		return nil, err
	}

	schemas := utils.FilterSlice(nodes, func(node *model.WorkflowNode) (*nbCore.NodeSchema, bool) {
		schema := datatypes.JSON{}
		tpl, ok := tplMap[node.WorkflowNodeID]
		n.schemaHelper.handleSchema(ctx, materialNodes, resourceNodeTemplates, tpl.Schema)
		if ok {
			var defaultValue map[string]any
			schema, defaultValue = n.schemaHelper.handleSchema(ctx, materialNodes, resourceNodeTemplates, tpl.Schema)
			if len(node.Param) == 0 && len(defaultValue) != 0 {
				node.Param, _ = json.Marshal(defaultValue)
			} else if len(node.Param) != 0 && len(defaultValue) != 0 {
				paramMap := make(map[string]any)
				if err := json.Unmarshal(node.Param, &paramMap); err == nil {
					maps.Copy(paramMap, defaultValue)
					node.Param, _ = json.Marshal(paramMap)
				}
			}
		}

		return &nbCore.NodeSchema{
			UUID:   node.UUID,
			Name:   node.Name,
			Schema: schema,
			Param:  node.Param,
		}, ok
	})

	return &nbCore.NotebookSchemaResp{
		UUID:        req.UUID,
		NodeSchemas: schemas,
	}, nil
}

func (n *notebookImpl) NotebookDetail(ctx context.Context, req *nbCore.NotebookDetailReq) (*nbCore.NotebookDetailResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return nil, code.UnLogin
	}

	// 获取 notebook 记录并校验权限
	notebook, err := n.notebookStore.GetNotebookByUUID(ctx, req.UUID)
	if err != nil {
		return nil, err
	}

	// 查询 notebook 下的分组
	groups, err := n.notebookStore.GetNotebookGroups(ctx, notebook.ID)
	if err != nil {
		return nil, err
	}

	// 获取 workflow UUID
	workflowUUID := n.notebookStore.ID2UUID(ctx, &model.Workflow{}, notebook.WorkflowID)[notebook.WorkflowID]
	// 获取用户信息
	userData, err := n.accountClient.GetUserInfo(ctx, notebook.UserID)
	if err != nil {
		logger.Errorf(ctx, "NotebookDetail GetUserInfo err: %+v", err)
	}
	var sampleUUIDs []uuid.UUID
	groupIDs := utils.FilterSlice(groups, func(g *model.NotebookGroup) (int64, bool) {
		sampleUUIDs = utils.AppendUniqSlice(sampleUUIDs, g.SampleUUIDs...)
		return g.ID, true
	})
	groupMap := utils.Slice2Map(groups, func(g *model.NotebookGroup) (int64, *model.NotebookGroup) {
		return g.ID, g
	})

	sampleData := make([]*model.Sample, 0, len(sampleUUIDs))
	if err := n.notebookStore.FindDatas(ctx, &sampleData, map[string]any{
		"uuid": sampleUUIDs,
	}, "uuid", "name"); err != nil {
		return nil, err
	}
	sampleDataMap := utils.Slice2Map(sampleData, func(s *model.Sample) (uuid.UUID, string) {
		return s.UUID, s.Name
	})

	// 查询参数记录
	params, err := n.notebookStore.GetNotebookParamsByGroupIDs(ctx, groupIDs)
	if err != nil {
		return nil, err
	}

	groupParamMap := utils.Slice2MapSlice(params, func(p *model.NotebookParam) (int64, *model.NotebookParam, bool) {
		return p.NotebookGroupID, p, true
	})

	// 构建 workflow_node_id -> uuid 映射
	workflowNodeIDs := utils.FilterUniqSlice(params, func(p *model.NotebookParam) (int64, bool) {
		return p.WorkflowNodeID, p.WorkflowNodeID > 0
	})

	// 查询结果集
	workflowTasks := make([]*model.WorkflowTask, 0, len(groupIDs))
	if err = n.notebookStore.FindDatas(ctx, &workflowTasks, map[string]any{
		"notebook_group_id": groupIDs,
	}, "id", "notebook_group_id"); err != nil {
		return nil, err
	}

	taskIDs := utils.FilterSlice(workflowTasks, func(t *model.WorkflowTask) (int64, bool) {
		return t.ID, true
	})

	workflowNotebookGroupTaskMap := utils.Slice2Map(workflowTasks, func(t *model.WorkflowTask) (int64, int64) {
		return t.ID, t.NotebookGroupID
	})

	workflowNodeJob := make([]*model.WorkflowNodeJob, 0, len(groupIDs))
	if err = n.notebookStore.FindDatas(ctx, &workflowNodeJob, map[string]any{
		"workflow_task_id": taskIDs,
	}, "id", "workflow_task_id", "node_id", "status", "return_info"); err != nil {
		return nil, err
	}

	workflowTaskMap := utils.Slice2MapSlice(workflowNodeJob, func(j *model.WorkflowNodeJob) (int64, *model.WorkflowNodeJob, bool) {
		return j.WorkflowTaskID, j, true
	})
	workflowTaskNodeMap := make(map[int64]map[int64]*model.WorkflowNodeJob)
	for taskID, jobs := range workflowTaskMap {
		notebookGroupID, ok := workflowNotebookGroupTaskMap[taskID]
		if !ok {
			continue
		}

		workflowTaskNodeMap[notebookGroupID] = utils.Slice2Map(jobs, func(j *model.WorkflowNodeJob) (int64, *model.WorkflowNodeJob) {
			return j.NodeID, j
		})
	}

	workflowNodeID2UUID := n.notebookStore.ID2UUID(ctx, &model.WorkflowNode{}, workflowNodeIDs...)
	detail := make([]*nbCore.NotebookGroup, len(groups))
	for index, groupID := range groupIDs {
		groupParams := groupParamMap[groupID]
		sort.Slice(groupParams, func(i, j int) bool {
			return groupParams[i].ID < groupParams[j].ID
		})

		groupData, _ := groupMap[groupID]

		jobs, ok := workflowTaskNodeMap[groupID]
		detail[index] = &nbCore.NotebookGroup{
			Samples: utils.FilterSlice(groupData.SampleUUIDs, func(u uuid.UUID) (*nbCore.SampleInfo, bool) {
				name, ok := sampleDataMap[u]
				return &nbCore.SampleInfo{
					SampleUUID: u,
					Name:       name,
				}, ok
			}),

			Params: utils.FilterSlice(groupParams, func(p *model.NotebookParam) (*nbCore.NotebookDetailItem, bool) {
				var result *datatypes.JSONType[model.ReturnInfo]
				if ok {
					if tmp, ok := jobs[p.WorkflowNodeID]; ok {
						result = &tmp.ReturnInfo
					}
				}

				var sampleParam datatypes.JSONSlice[model.SampleParam]
				if len(p.SampleValue) != 0 {
					sampleParam = p.SampleValue
				} else {
					sampleParam = datatypes.NewJSONSlice([]model.SampleParam{})
				}

				return &nbCore.NotebookDetailItem{
					NodeUUID:     workflowNodeID2UUID[p.WorkflowNodeID],
					Param:        p.Param,
					SampleParams: sampleParam,
					Result:       result,
				}, true
			}),
		}
	}

	return &nbCore.NotebookDetailResp{
		UUID:           notebook.UUID,
		Name:           notebook.Name,
		Status:         notebook.Status,
		WorkflowUUID:   workflowUUID,
		UserName:       userData.Name,
		DisplayName:    userData.DisplayName,
		NotebookGroups: detail,
	}, nil
}

func (n *notebookImpl) CreateSample(ctx context.Context, req *nbCore.SampleReq) (*nbCore.SampleResp, error) {
	labID := n.notebookStore.UUID2ID(ctx, &model.Laboratory{}, req.LabUUID)[req.LabUUID]
	if labID == 0 {
		return nil, code.LabNotFound
	}

	if len(req.Items) == 0 {
		return nil, code.ParamErr
	}

	materialUUIDs := utils.FilterUniqSlice(req.Items, func(i *nbCore.SampleItem) (uuid.UUID, bool) {
		return i.MaterialNodeUUID, !i.MaterialNodeUUID.IsNil()
	})

	if len(materialUUIDs) != len(req.Items) {
		return nil, code.ParamErr.WithMsg("duplicate material uuid")
	}

	materialUUIDMap := n.notebookStore.UUID2ID(ctx, &model.MaterialNode{}, materialUUIDs...)
	if len(materialUUIDMap) != len(materialUUIDs) {
		return nil, code.ParamErr.WithMsg("material node not exist")
	}

	sampleDatas := utils.FilterSlice(req.Items, func(i *nbCore.SampleItem) (*model.Sample, bool) {
		return &model.Sample{
			LabID:            labID,
			Name:             i.Name,
			MaterialNodeID:   materialUUIDMap[i.MaterialNodeUUID],
			MaterialNodeUUID: i.MaterialNodeUUID,
		}, true
	})

	if err := n.notebookStore.ExecTx(ctx, func(txCtx context.Context) error {
		if err := n.notebookStore.CreateSample(txCtx, sampleDatas); err != nil {
			return err
		}

		materialDatas := utils.FilterSlice(sampleDatas, func(s *model.Sample) (*model.MaterialNode, bool) {
			return &model.MaterialNode{
				BaseModel: model.BaseModel{UUID: s.MaterialNodeUUID},
				Extra: datatypes.JSONMap(map[string]any{
					"sample_uuid": s.UUID,
					"name":        s.Name,
				}),
			}, true
		})

		_, err := n.materialStore.UpsertMaterialNode(txCtx, materialDatas, []string{"uuid"}, []string{"uuid"}, "extra")
		return err
	}); err != nil {
		return nil, err
	}

	return &nbCore.SampleResp{
		Items: utils.FilterSlice(sampleDatas, func(s *model.Sample) (*nbCore.SampleItem, bool) {
			return &nbCore.SampleItem{
				MaterialNodeUUID: s.MaterialNodeUUID,
				Name:             s.Name,
				SampleUUID:       s.UUID,
			}, true
		}),
	}, nil
}

// GetNotebookBySample 根据样品获取 notebook
func (n *notebookImpl) GetNotebookBySample(ctx context.Context, req *nbCore.GetNotebookBySampleReq) (*nbCore.GetNotebookBySampleResp, error) {
	// 1. 根据 sample_name 直接获取 sample_id 列表
	sampleIDs, err := n.notebookStore.GetSampleIDsByName(ctx, req.SampleName)
	if err != nil {
		logger.Errorf(ctx, "GetNotebookBySample GetSampleIDsByName err: %+v", err)
		return nil, err
	}

	if len(sampleIDs) == 0 {
		return &nbCore.GetNotebookBySampleResp{Res: []*nbCore.NotebookBySampleItem{}}, nil
	}

	// 2. 查询 workflow_node_job_sample 表，使用 DISTINCT 获取唯一的 job_id
	jobIDs, err := n.notebookStore.GetDistinctJobIDsBySampleIDs(ctx, sampleIDs)
	if err != nil {
		logger.Errorf(ctx, "GetNotebookBySample GetDistinctJobIDsBySampleIDs err: %+v", err)
		return nil, err
	}

	if len(jobIDs) == 0 {
		return &nbCore.GetNotebookBySampleResp{Res: []*nbCore.NotebookBySampleItem{}}, nil
	}

	// 3. 查询 workflow_node_job 表
	jobs := make([]*model.WorkflowNodeJob, 0)
	if err := n.notebookStore.FindDatas(ctx, &jobs, map[string]any{
		"id": jobIDs,
	}); err != nil {
		logger.Errorf(ctx, "GetNotebookBySample query workflow_node_job err: %+v", err)
		return nil, err
	}

	// 获取所有 workflow_task_id
	workflowTaskIDs := utils.FilterUniqSlice(jobs, func(j *model.WorkflowNodeJob) (int64, bool) {
		return j.WorkflowTaskID, true
	})

	// 4. 查询 workflow_task 表
	tasks := make([]*model.WorkflowTask, 0)
	if err := n.notebookStore.FindDatas(ctx, &tasks, map[string]any{
		"id": workflowTaskIDs,
	}); err != nil {
		logger.Errorf(ctx, "GetNotebookBySample query workflow_task err: %+v", err)
		return nil, err
	}

	// 从已查询的 tasks 中提取唯一的 notebook_group_id
	notebookGroupIDs := utils.FilterUniqSlice(tasks, func(t *model.WorkflowTask) (int64, bool) {
		return t.NotebookGroupID, true
	})

	// 5. 查询 notebook_group 表
	notebookGroups := make([]*model.NotebookGroup, 0)
	if err := n.notebookStore.FindDatas(ctx, &notebookGroups, map[string]any{
		"id": notebookGroupIDs,
	}); err != nil {
		logger.Errorf(ctx, "GetNotebookBySample query notebook_group err: %+v", err)
		return nil, err
	}

	// 从已查询的 notebookGroups 中提取唯一的 notebook_id
	notebookIDs := utils.FilterUniqSlice(notebookGroups, func(ng *model.NotebookGroup) (int64, bool) {
		return ng.NotebookID, true
	})

	// 6. 将 notebook_id 转换为 uuid
	notebookUUIDMap := n.notebookStore.ID2UUID(ctx, &model.Notebook{}, notebookIDs...)

	// 7. 从已查询的 notebookGroups 中构建 notebook_group_id 到 uuid 的映射
	notebookGroupUUIDMap := utils.Slice2Map(notebookGroups, func(ng *model.NotebookGroup) (int64, uuid.UUID) {
		return ng.ID, ng.UUID
	})

	// 8. 构建 job_id 到 workflow_task_id 的映射
	jobIDToTaskIDMap := utils.Slice2Map(jobs, func(j *model.WorkflowNodeJob) (int64, int64) {
		return j.ID, j.WorkflowTaskID
	})

	// 9. 构建 workflow_task_id 到 notebook_group_id 的映射
	taskIDToGroupIDMap := utils.Slice2Map(tasks, func(t *model.WorkflowTask) (int64, int64) {
		return t.ID, t.NotebookGroupID
	})

	// 10. 构建 notebook_group_id 到 notebook_id 的映射
	groupIDToNotebookIDMap := utils.Slice2Map(notebookGroups, func(ng *model.NotebookGroup) (int64, int64) {
		return ng.ID, ng.NotebookID
	})

	// 11. 组装结果：通过 job -> task -> group -> notebook 的链路
	// 按 notebook_uuid 和 notebook_group_uuid 分组，聚合 return_info
	// 结构：notebook_uuid -> notebook_group_uuid -> []return_info
	type groupKey struct {
		NotebookUUID      uuid.UUID
		NotebookGroupUUID uuid.UUID
	}
	groupKeyToReturnInfosMap := utils.Slice2MapSlice(jobs, func(job *model.WorkflowNodeJob) (groupKey, *model.ReturnInfo, bool) {
		returnInfoData := job.ReturnInfo.Data()
		returnInfo := &returnInfoData

		taskID, hasTaskID := jobIDToTaskIDMap[job.ID]
		if !hasTaskID {
			return groupKey{}, nil, false
		}

		groupID, hasGroupID := taskIDToGroupIDMap[taskID]
		if !hasGroupID {
			return groupKey{}, nil, false
		}

		notebookID, hasNotebookID := groupIDToNotebookIDMap[groupID]
		if !hasNotebookID {
			return groupKey{}, nil, false
		}

		notebookUUID, hasNotebookUUID := notebookUUIDMap[notebookID]
		if !hasNotebookUUID {
			return groupKey{}, nil, false
		}

		notebookGroupUUID, hasNotebookGroupUUID := notebookGroupUUIDMap[groupID]
		if !hasNotebookGroupUUID {
			return groupKey{}, nil, false
		}

		// 按 notebook_uuid 和 notebook_group_uuid 分组，聚合 return_info
		key := groupKey{
			NotebookUUID:      notebookUUID,
			NotebookGroupUUID: notebookGroupUUID,
		}
		return key, returnInfo, true
	})

	// 13. 转换为结果数组：先按 notebook_uuid 分组，再按 notebook_group_uuid 分组
	notebookUUIDToSampleDataMap := make(map[uuid.UUID][]*nbCore.SampleData)
	utils.RangeMap(groupKeyToReturnInfosMap, func(key groupKey, returnInfos []*model.ReturnInfo) bool {
		notebookUUIDToSampleDataMap[key.NotebookUUID] = append(
			notebookUUIDToSampleDataMap[key.NotebookUUID],
			&nbCore.SampleData{
				ReturnInfo:        returnInfos,
				NotebookGroupUUID: key.NotebookGroupUUID,
			},
		)
		return true
	})

	res := utils.Map2Slice(notebookUUIDToSampleDataMap, func(notebookUUID uuid.UUID, sampleData []*nbCore.SampleData) (*nbCore.NotebookBySampleItem, bool) {
		return &nbCore.NotebookBySampleItem{
			NotebookUUID: notebookUUID,
			SampleData:   sampleData,
		}, true
	})

	return &nbCore.GetNotebookBySampleResp{
		Res: res,
	}, nil
}

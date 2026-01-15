package notebook

/*
 组任务排队执行，执行后记录结果
*/

import (
	// 外部依赖
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"maps"
	"sort"
	"sync"
	"sync/atomic"
	"time"

	melody "github.com/olahol/melody"
	ants "github.com/panjf2000/ants/v2"
	gjson "github.com/tidwall/gjson"
	sjson "github.com/tidwall/sjson"
	datatypes "gorm.io/datatypes"
	
	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	engine "github.com/scienceol/opensdl/service/pkg/core/schedule/engine"
	lab "github.com/scienceol/opensdl/service/pkg/core/schedule/lab"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	eStore "github.com/scienceol/opensdl/service/pkg/repo/environment"
	model "github.com/scienceol/opensdl/service/pkg/model"
	notebook "github.com/scienceol/opensdl/service/pkg/repo/notebook"
	wfl "github.com/scienceol/opensdl/service/pkg/repo/workflow"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

type stepFunc func(ctx context.Context) error

type notebookEngine struct {
	sessionCtx context.Context
	job        *engine.NotebookInfo
	cancel     context.CancelFunc
	cancelCtx  context.Context
	baseCtx    context.Context
	session    *melody.Session

	envStore      repo.LaboratoryRepo
	workflowStore repo.WorkflowRepo
	notebookStore repo.NotebookRepo

	nodes           []*model.WorkflowNode           // 所有节点
	edges           []*model.WorkflowEdge           // 所有边
	handles         []*model.WorkflowHandleTemplate // 所有 handles
	params          *NotebookData                   // notebook 数据
	currentTaskUUID uuid.UUID                       // 当前正在运行的 task uuid
	currentTaskID   int64                           // 当前 task id
	currentGroupID  int64

	jobMap          map[uuid.UUID]*model.WorkflowNodeJob // 所有的 job map
	nodeMap         map[int64]*model.WorkflowNodeJob     // 所有的 node 对应的运行结果
	nodeParentEdges map[int64][]*engine.HandlePair       // 节点对应的所有 parent edge

	dependencies map[*model.WorkflowNode]map[*model.WorkflowNode]struct{} // dag 图依赖关系

	pools     *ants.Pool
	wg        sync.WaitGroup
	stepFuncs []stepFunc

	sandbox repo.Sandbox

	actionStatus sync.Map
}

func NewNotebookTask(ctx context.Context, param *engine.NotebookInfo) engine.Task {
	pools, _ := ants.NewPool(5,
		ants.WithExpiryDuration(10*time.Second))
	baseCtx := context.Background()
	cancelCtx, cancel := context.WithCancel(baseCtx)

	d := &notebookEngine{
		sessionCtx:      ctx,
		session:         param.Session,
		cancel:          cancel,
		cancelCtx:       cancelCtx,
		baseCtx:         baseCtx,
		envStore:        eStore.New(),
		workflowStore:   wfl.New(),
		notebookStore:   notebook.New(),
		dependencies:    make(map[*model.WorkflowNode]map[*model.WorkflowNode]struct{}),
		pools:           pools,
		wg:              sync.WaitGroup{},
		jobMap:          make(map[uuid.UUID]*model.WorkflowNodeJob),
		nodeMap:         make(map[int64]*model.WorkflowNodeJob),
		nodeParentEdges: make(map[int64][]*engine.HandlePair),
		sandbox:         param.Sandbox,
		job: &engine.NotebookInfo{
			LabUUID:      param.LabUUID,
			LabID:        param.LabID,
			UserID:       param.UserID,
			NotebookUUID: param.NotebookUUID,
		},
	}
	d.stepFuncs = append(d.stepFuncs,
		d.checkTaskStatus, // 检查任务状态
		d.loadData,        // 加载运行数据
		d.buildTask,       // 构建任务
		d.runAllNodes,     // 运行任务
	)

	return d
}

func (d *notebookEngine) checkTaskStatus(ctx context.Context) error {
	notebookData := &model.Notebook{}
	if err := d.notebookStore.GetData(ctx, notebookData, map[string]any{
		"uuid": d.job.NotebookUUID,
	}, "id", "uuid", "status", "workflow_id"); err != nil {
		return code.NotebookNotExistErr
	}

	if notebookData.Status != model.NotebookStatusInit {
		return code.NotebookNotInitErr
	}

	notebookData.Status = model.NotebookStatusPending
	notebookData.StartTime = time.Now()
	if err := d.notebookStore.UpdateData(ctx, notebookData, map[string]any{
		"id": notebookData.ID,
	}, "status", "start_time"); err != nil {
		return err
	}

	d.job.WorkflowID = notebookData.WorkflowID
	d.job.NotebookID = notebookData.ID
	return nil
}

func (d *notebookEngine) loadData(ctx context.Context) error {
	// 获取工作流
	wk := &model.Workflow{}
	err := d.workflowStore.GetData(ctx, wk, map[string]any{
		"id": d.job.WorkflowID,
	})
	if err != nil {
		return err
	}

	// 加载所有工作流节点数据
	allNodes, err := d.workflowStore.GetWorkflowNodes(ctx, map[string]any{
		"workflow_id": wk.ID,
		"type": []model.WorkflowNodeType{
			model.WorkflowNodeILab,
			model.WorkflowPyScript,
		},
	})
	if err != nil {
		return err
	}

	// 过滤检查可执行节点
	nodes, err := utils.FilterSliceWithErr(allNodes, func(node *model.WorkflowNode) ([]*model.WorkflowNode, bool, error) {
		if node.Type == model.WorkflowNodeGroup || node.Disabled {
			return nil, false, nil
		}

		if node.Type == model.WorkflowNodeILab {
			if node.DeviceName == nil || *node.DeviceName == "" {
				return nil, false, code.WorkflowNodeNoDeviceName
			}

			if node.ActionName == "" {
				return nil, false, code.WorkflowNodeNoActionName
			}

			if node.ActionType == "" {
				return nil, false, code.WorkflowNodeNoActionType
			}
		} else {
			// 计算类型
			if node.Script == nil || *node.Script == "" {
				return nil, false, code.WorkflowNodeScriptEmtpyErr
			}
		}

		return []*model.WorkflowNode{node}, true, nil
	})
	if err != nil {
		return err
	}

	// 节点UUID查询边
	nodeUUIDs := utils.FilterSlice(nodes, func(node *model.WorkflowNode) (uuid.UUID, bool) {
		return node.UUID, true
	})

	edges, err := d.workflowStore.GetWorkflowEdges(ctx, nodeUUIDs)
	if err != nil {
		return err
	}

	edgeHandleUUIDs := make([]uuid.UUID, 0, 2*len(edges))
	utils.Range(edges, func(_ int, e *model.WorkflowEdge) bool {
		edgeHandleUUIDs = utils.AppendUniqSlice(edgeHandleUUIDs, e.SourceHandleUUID)
		edgeHandleUUIDs = utils.AppendUniqSlice(edgeHandleUUIDs, e.TargetHandleUUID)
		return true
	})

	handleTpls := make([]*model.WorkflowHandleTemplate, 0, len(edgeHandleUUIDs))
	if err := d.workflowStore.FindDatas(ctx, &handleTpls, map[string]any{
		"uuid": edgeHandleUUIDs,
	}); err != nil {
		return err
	}

	d.nodes = nodes
	d.edges = edges
	d.handles = handleTpls
	if d.params, err = d.getNotebookData(ctx); err != nil {
		return err
	}

	return nil
}

func (d *notebookEngine) getNotebookData(ctx context.Context) (*NotebookData, error) {
	nbData := &model.Notebook{}
	if err := d.notebookStore.GetData(ctx, nbData, map[string]any{
		"uuid": d.job.NotebookUUID,
	}); err != nil {
		return nil, err
	}

	groups := make([]*model.NotebookGroup, 0, 10)
	if err := d.notebookStore.FindDatas(ctx, &groups, map[string]any{
		"notebook_id": nbData.ID,
		"status":      model.NotebookStatusInit,
	}); err != nil {
		return nil, err
	}

	if len(groups) == 0 {
		return nil, code.NotebookParamEmptyErr
	}

	sort.Slice(groups, func(i, j int) bool {
		return groups[i].ID < groups[j].ID
	})

	groupIDs := utils.FilterSlice(groups, func(g *model.NotebookGroup) (int64, bool) {
		return g.ID, true
	})

	params := make([]*model.NotebookParam, 0, 10)
	if err := d.notebookStore.FindDatas(ctx, &params, map[string]any{
		"notebook_group_id": groupIDs,
	}); err != nil {
		return nil, err
	}

	paramMap := utils.Slice2MapSlice(params, func(p *model.NotebookParam) (int64, *model.NotebookParam, bool) {
		return p.NotebookGroupID, p, true
	})

	if len(paramMap) == 0 {
		return nil, code.NotebookParamEmptyErr
	}

	paramCount := len(params) / len(groups)
	for _, params := range paramMap {
		if len(params) != paramCount {
			return nil, code.NotebookParamCountErr
		}
	}

	var err error
	groupData := utils.FilterSlice(groups, func(g *model.NotebookGroup) (*NoteBookGroupData, bool) {
		paramSlice, ok := paramMap[g.ID]
		if !ok {
			err = code.NotebookParamEmptyErr
		}

		if err != nil {
			return nil, false
		}

		return &NoteBookGroupData{
			Group: g,
			Params: utils.Slice2Map(paramSlice, func(p *model.NotebookParam) (int64, *model.NotebookParam) {
				return p.WorkflowNodeID, p
			}),
		}, err == nil
	})

	return &NotebookData{
		NotebookID: nbData.ID,
		NotebookGroupIDs: utils.FilterSlice(groups, func(g *model.NotebookGroup) (int64, bool) {
			return g.ID, true
		}),
		NotebookGroupMap: utils.Slice2Map(groupData, func(g *NoteBookGroupData) (int64, *NoteBookGroupData) {
			return g.Group.ID, g
		}),
	}, nil
}

func (d *notebookEngine) buildTask(ctx context.Context) error {
	// 构建图关系
	nodeMap := utils.Slice2Map(d.nodes, func(node *model.WorkflowNode) (uuid.UUID, *model.WorkflowNode) {
		return node.UUID, node
	})

	nodeParentUUIDMap := make(map[uuid.UUID][]uuid.UUID)
	nodeChildrenUUIDMap := make(map[uuid.UUID][]uuid.UUID)

	for _, edge := range d.edges {
		// 目标节点的所有源节点
		nodeParentUUIDMap[edge.TargetNodeUUID] = append(nodeParentUUIDMap[edge.TargetNodeUUID], edge.SourceNodeUUID)
		// 源节点的所有目标节点
		nodeChildrenUUIDMap[edge.SourceNodeUUID] = append(nodeChildrenUUIDMap[edge.SourceNodeUUID], edge.TargetNodeUUID)
	}

	// 先检测循环
	if err := d.detectCycle(nodeChildrenUUIDMap); err != nil {
		return err
	}
	handleMap := utils.Slice2Map(d.handles, func(h *model.WorkflowHandleTemplate) (uuid.UUID, *model.WorkflowHandleTemplate) {
		return h.UUID, h
	})

	for _, node := range d.nodes {
		parentNodeMap := make(map[*model.WorkflowNode]struct{})
		d.findAllParents(nodeMap, nodeParentUUIDMap, node, parentNodeMap)
		d.dependencies[node] = parentNodeMap

		// 找出该节点的所有前向边
		leftEdges := utils.FilterSlice(d.edges, func(e *model.WorkflowEdge) (*model.WorkflowEdge, bool) {
			if node.UUID == e.TargetNodeUUID {
				return e, true
			}
			return nil, false
		})

		if config.Global().Dynamic().Schedule.TranslateNodeParam {
			var err error
			d.nodeParentEdges[node.ID], err = utils.FilterSliceErr(leftEdges, func(e *model.WorkflowEdge) (*engine.HandlePair, bool, error) {
				sourceHandle, ok := handleMap[e.SourceHandleUUID]
				if !ok {
					return nil, false, code.CanNotFoundWorkflowHandleErr.WithMsg(fmt.Sprintf("node id: %d, source uuid: %s", node.ID, e.SourceHandleUUID))
				}

				targetHandle, ok := handleMap[e.TargetHandleUUID]
				if !ok {
					return nil, false, code.CanNotFoundWorkflowHandleErr.WithMsg(fmt.Sprintf("node id: %d, target uuid: %s", node.ID, e.TargetHandleUUID))
				}

				pair := &engine.HandlePair{
					SourceHandle: sourceHandle,
					TargetHandle: targetHandle,
				}

				sourceNode, ok := nodeMap[e.SourceNodeUUID]
				if ok {
					pair.SourceNode = sourceNode
				}
				// 不存在的情况是父节点被禁用了

				return pair, true, nil
			})
			if err != nil {
				return err
			}
		}
	}
	return nil
}

// 使用DFS检测循环
func (d *notebookEngine) detectCycle(nodeChildrenUUIDMap map[uuid.UUID][]uuid.UUID) error {
	visited := make(map[uuid.UUID]bool)
	recStack := make(map[uuid.UUID]bool)

	for _, node := range d.nodes {
		if !visited[node.UUID] {
			if d.dfsDetectCycle(node.UUID, nodeChildrenUUIDMap, visited, recStack) {
				return code.WorkflowHasCircularErr
			}
		}
	}

	return nil
}

func (d *notebookEngine) dfsDetectCycle(nodeUUID uuid.UUID,
	nodeChildrenUUIDMap map[uuid.UUID][]uuid.UUID, visited, recStack map[uuid.UUID]bool,
) bool {
	visited[nodeUUID] = true
	recStack[nodeUUID] = true

	// 查找所有子节点
	if children, exists := nodeChildrenUUIDMap[nodeUUID]; exists {
		for _, child := range children {
			if !visited[child] {
				if d.dfsDetectCycle(child, nodeChildrenUUIDMap, visited, recStack) {
					return true
				}
			} else if recStack[child] {
				return true // 发现循环
			}
		}
	}

	recStack[nodeUUID] = false
	return false
}

// 修复后的findAllParents方法，支持多个父节点
func (d *notebookEngine) findAllParents(nodeMap map[uuid.UUID]*model.WorkflowNode,
	nodeParentUUIDMap map[uuid.UUID][]uuid.UUID, node *model.WorkflowNode, parentMap map[*model.WorkflowNode]struct{},
) {
	if node == nil {
		return
	}

	// 查找该节点的所有父节点
	if sources, exists := nodeParentUUIDMap[node.UUID]; exists {
		for _, sourceUUID := range sources {
			parentNode, ok := nodeMap[sourceUUID]
			if !ok {
				continue // 父节点不存在
			}

			// 避免重复访问
			if _, exists := parentMap[parentNode]; !exists {
				parentMap[parentNode] = struct{}{}
				// 递归查找父节点的父节点
				d.findAllParents(nodeMap, nodeParentUUIDMap, parentNode, parentMap)
			}
		}
	}
}

// 运行入口
func (d *notebookEngine) Run() error {
	var err error

	for _, step := range d.stepFuncs {
		if err = step(d.baseCtx); err != nil {
			break
		}
	}

	d.wg.Wait()
	d.cancel()
	return err
}

func (d *notebookEngine) Stop() error {
	data := lab.EdgeData[*engine.CancelTask]{
		EdgeMsg: lab.EdgeMsg{
			Action: lab.CancelTask,
		},
		Data: &engine.CancelTask{
			TaskID: d.currentTaskUUID,
		},
	}
	b, _ := json.Marshal(data)
	d.session.Write(b)

	d.cancel()
	d.wg.Wait()

	if d.pools != nil {
		d.pools.Release()
	}

	return nil
}

func (d *notebookEngine) runAllNodes(ctx context.Context) error {
	nbData := &model.Notebook{}
	nbData.Status = model.NotebookStatusRunnig
	if err := d.notebookStore.UpdateData(ctx, nbData, map[string]any{
		"id": d.job.NotebookID,
	}, "status"); err != nil {
		return err
	}

	defer func() {
		if d.pools != nil {
			d.pools.Release()
		}
	}()

	var runErr error
outerLoop:
	for _, groupID := range d.params.NotebookGroupIDs {
		select {
		case <-d.cancelCtx.Done():
			runErr = code.JobCanceled
			break outerLoop
		default:
		}
		param, _ := d.params.NotebookGroupMap[groupID]
		task := &model.WorkflowTask{
			LabID:           d.job.LabID,
			WorkflowID:      d.job.WorkflowID,
			UserID:          d.job.UserID,
			NotebookGroupID: param.Group.ID,
			Status:          model.WorkflowTaskStatusPending,
			FinishedTime:    time.Time{},
		}
		if err := d.workflowStore.CreateWorkflowTask(ctx, task); err != nil {
			logger.Errorf(ctx, "notebookEngine.runAllNodes CreateWorkflowTask group id: %d, fail err: %+v", groupID, err)
			break
		}

		notebookGroupData := &model.NotebookGroup{
			Status:    model.NotebookStatusRunnig,
			StartTime: time.Now(),
		}

		if err := d.notebookStore.UpdateData(ctx, notebookGroupData,
			map[string]any{"id": groupID}, "status", "start_time"); err != nil {
			logger.Errorf(ctx, "notebookEngine.runAllNodes UpdateData NotebookGroup fail group id: %d, err: %+v", groupID, err)
			break
		}

		d.currentTaskUUID = task.UUID
		d.currentTaskID = task.ID
		d.currentGroupID = groupID
		runErr = d.runNotebookGroup(ctx, groupID, task)
		notebookGroupData.FinishedTime = time.Now()
		if runErr != nil {
			notebookGroupData.Status = model.NotebookStatusFail
		} else {
			notebookGroupData.Status = model.NotebookStatusSuccess
		}
		if err := d.notebookStore.UpdateData(ctx, notebookGroupData,
			map[string]any{"id": groupID}, "status", "finished_time"); err != nil {
			logger.Errorf(ctx, "notebookEngine.runAllNodes UpdateData fail err: %+v", err)
			break
		}
		if runErr != nil {
			logger.Errorf(ctx, "notebookEngine.runAllNodes runNotebookGroup fail group id: %d, err: %+v", groupID, runErr)
			break
		}
	}

	nbData.Status = model.NotebookStatusSuccess
	nbData.FinishedTime = time.Now()
	if runErr != nil {
		nbData.Status = model.NotebookStatusFail
	}

	if err := d.notebookStore.UpdateData(ctx, nbData,
		map[string]any{"id": d.job.NotebookID}, "status", "finished_time"); err != nil {
		logger.Errorf(ctx, "notebookEngine.runAllNodes UpdateData fail err: %+v", err)
	}

	return runErr
}

func (d *notebookEngine) runNotebookGroup(ctx context.Context, groupID int64, task *model.WorkflowTask) error {
	var hasError atomic.Bool
	var firstError atomic.Value

	dependencies := d.copyDependencies()
	for {
		if len(dependencies) == 0 {
			// 该函数出口
			return nil
		}

		select {
		case <-d.cancelCtx.Done():
			return code.JobCanceled
		default:
		}

		noDepNodes := make([]*model.WorkflowNode, 0, 10)
		nodeJobs := make([]*model.WorkflowNodeJob, 0, 10)
		for node, nodeDependences := range dependencies {
			if len(nodeDependences) > 0 {
				continue
			}

			noDepNodes = append(noDepNodes, node)
			nodeJobs = append(nodeJobs, &model.WorkflowNodeJob{
				LabID:          d.job.LabID,
				WorkflowTaskID: task.ID,
				NodeID:         node.ID,
				Status:         model.WorkflowJobPending,
			})
		}

		if err := d.workflowStore.CreateJobs(ctx, nodeJobs); err != nil {
			logger.Errorf(ctx, "runNotebookGroup.CreateJobs fail group id: %d, task id: %d, err: %+v", groupID, task.ID, err)
			return err
		}

		for _, job := range nodeJobs {
			d.jobMap[job.UUID] = job
			d.nodeMap[job.NodeID] = job
		}

		for index, node := range noDepNodes {
			newNode := node
			newIndex := index
			d.wg.Add(1)

			if err := d.pools.Submit(func() {
				defer d.wg.Done()

				if err := utils.SafelyRun(func() {
					select {
					case <-d.cancelCtx.Done():
						if !hasError.Load() {
							firstError.Store(code.JobCanceled)
							hasError.Store(true)
						}
						return
					default:
					}

					if err := d.runNode(ctx, newNode, nodeJobs[newIndex]); err != nil {
						if !errors.Is(err, code.JobCanceled) {
							logger.Errorf(ctx, "node run fail group id: %d, node id: %d, err: %+v", groupID, newNode.ID, err)
						}

						if !hasError.Load() {
							firstError.Store(err)
							hasError.Store(true)
							d.cancel()
							return
						}
					}
				}); err != nil {
					// panic 才会输出该日志
					logger.Errorf(ctx, "run all node fail group id: %d, SafelyRun err: %+v", groupID, err)
				}
			}); err != nil {
				// 携程池函数获取携程数据出错的时候才会报错
				logger.Errorf(ctx, "run all node submit run node fail group id: %d, err: %+v", groupID, err)
			}
		}

		d.wg.Wait()

		if hasError.Load() {
			return firstError.Load().(error)
		}

		// 移除依赖关系
		for _, runnedNode := range noDepNodes {
			delete(dependencies, runnedNode)
			for _, nodeDependences := range dependencies {
				delete(nodeDependences, runnedNode)
			}
		}
	}
}

func (d *notebookEngine) copyDependencies() map[*model.WorkflowNode]map[*model.WorkflowNode]struct{} {
	copyData := make(map[*model.WorkflowNode]map[*model.WorkflowNode]struct{})
	for k, v := range d.dependencies {
		innerMap := make(map[*model.WorkflowNode]struct{}, len(v))
		maps.Copy(innerMap, v)
		copyData[k] = innerMap
	}
	return copyData
}

func (d *notebookEngine) changeParam(_ context.Context, node *model.WorkflowNode) error {
	param, ok := d.params.NotebookGroupMap[d.currentGroupID]
	if !ok {
		return code.NotebookChangeParamErr
	}

	paramData, ok := param.Params[node.ID]
	if !ok {
		return nil
	}

	node.Param = paramData.Param

	return nil
}

func (d *notebookEngine) parsePreNodeParam(_ context.Context, node *model.WorkflowNode) error {
	dynamicConf := config.Global().Dynamic()
	if !dynamicConf.Schedule.TranslateNodeParam {
		return nil
	}

	pairs, ok := d.nodeParentEdges[node.ID]
	if !ok || len(pairs) == 0 {
		return nil
	}

	for _, p := range pairs {
		// 无父节点
		if p.SourceNode == nil {
			continue
		}

		if p.SourceHandle == nil || p.SourceHandle.DataKey == "" {
			continue
		}

		if p.TargetHandle == nil || p.TargetHandle.DataKey == "" {
			continue
		}

		if p.SourceHandle.DataSource != "executor" ||
			p.SourceHandle.HandleKey == "ready" {
			continue
		}

		job, ok := d.nodeMap[p.SourceNode.ID]
		if !ok {
			return code.CanNotGetParentJobErr.WithMsg(
				fmt.Sprintf("parent node id: %d, node id: %d", p.SourceNode.ID, node.ID))
		}

		var err error
		retValueB, err := json.Marshal(job.ReturnInfo.Data().ReturnValue)
		if err != nil {
			return code.DataNotMapAnyTypeErr
		}
		res := gjson.Get(string(retValueB), p.SourceHandle.DataKey)
		if !res.Exists() {
			return code.ValueNotExistErr
		}

		jsonStr, err := sjson.Set(string(node.Param), p.TargetHandle.DataKey, res.Value())
		if err != nil {
			return code.UpdateNodeErr
		}

		node.Param = datatypes.JSON(jsonStr)
	}

	return nil
}

func (d *notebookEngine) runNode(ctx context.Context, node *model.WorkflowNode, job *model.WorkflowNodeJob) error {
	// 交换节点参数
	if err := d.changeParam(ctx, node); err != nil {
		return err
	}

	if err := d.parsePreNodeParam(ctx, node); err != nil {
		return err
	}

	var err error
	defer func() {
		jobStatus := model.WorkflowJobFailed
		if err != nil {
			jobStatus = model.WorkflowJobFailed
			if errors.Is(err, code.JobCanceled) {
				jobStatus = model.WorkflowJobCanceled
			}

			if errors.Is(err, code.JobTimeoutErr) {
				jobStatus = model.WorkflowJobTimeout
			} else {
				jobStatus = model.WorkflowJobFailed
			}
		} else {
			jobStatus = model.WorkflowJobSuccess
		}

		d.updateJob(ctx, jobStatus, job.ID)
	}()

	// 查询 action 是否可以执行
	if node.Type == model.WorkflowNodeILab {
		err = d.queryAction(ctx, node, job)
		if err != nil {
			return err
		}
	}

	err = d.execNodeAction(ctx, node, job)
	if err != nil {
		return err
	}

	if node.Type == model.WorkflowPyScript {
		return err
	}

	key := engine.ActionKey{
		Type:       engine.JobCallbackStatus,
		TaskUUID:   d.currentTaskUUID,
		JobID:      job.UUID,
		DeviceID:   *node.DeviceName,
		ActionName: node.ActionName,
	}

	d.InitDeviceActionStatus(ctx, key, time.Now().Add(20*time.Second), false)
	err = d.callbackAction(ctx, key, job)

	return err
}

func (d *notebookEngine) queryAction(ctx context.Context, node *model.WorkflowNode, job *model.WorkflowNodeJob) error {
	if node.Type == model.WorkflowPyScript {
		return nil
	}

	key := engine.ActionKey{
		Type:     engine.QueryActionStatus,
		TaskUUID: d.currentTaskUUID,
		JobID:    job.UUID,
		DeviceID: utils.SafeValue(func() string {
			return *node.DeviceName
		}, ""),
		ActionName: node.ActionName,
	}
	d.InitDeviceActionStatus(ctx, key, time.Now().Add(time.Second*20), false)
	if err := d.sendQueryAction(ctx, node, job); err != nil {
		return err
	}

	for {
		select {
		case <-d.cancelCtx.Done():
			return code.JobCanceled
		default:
		}

		time.Sleep(time.Millisecond * 500)
		value, exist := d.GetDeviceActionStatus(ctx, key)
		if !exist {
			return code.QueryJobStatusKeyNotExistErr
		}

		if value.Free {
			d.DelStatus(ctx, key)
			return nil
		}

		if value.Timestamp.Unix() < time.Now().Unix() {
			return code.JobTimeoutErr
		}
	}
}

func (d *notebookEngine) sendQueryAction(_ context.Context, node *model.WorkflowNode, job *model.WorkflowNodeJob) error {
	if d.session.IsClosed() {
		return code.EdgeConnectClosedErr
	}

	data := lab.EdgeData[engine.ActionKey]{
		EdgeMsg: lab.EdgeMsg{
			Action: lab.QueryActionStatus,
		},
		Data: engine.ActionKey{
			TaskUUID:   d.currentTaskUUID,
			JobID:      job.UUID,
			DeviceID:   *node.DeviceName,
			ActionName: node.ActionName,
		},
	}

	bData, _ := json.Marshal(data)
	return d.session.Write(bData)
}

func (d *notebookEngine) execNodeAction(ctx context.Context, node *model.WorkflowNode, job *model.WorkflowNodeJob) error {
	switch node.Type {
	case model.WorkflowNodeILab:
		return d.sendAction(ctx, node, job)
	case model.WorkflowPyScript:
		return d.execScript(ctx, node, job)
	default:
		return code.UnknownWorkflowNodeTypeErr
	}
}

func (d *notebookEngine) execScript(ctx context.Context, node *model.WorkflowNode, job *model.WorkflowNodeJob) error {
	inputs := map[string]any{}
	err := json.Unmarshal(node.Param, &inputs)
	returnInfo := model.ReturnInfo{
		Suc:         false,
		Error:       "",
		ReturnValue: nil,
	}

	ret, errMsg, err := d.sandbox.ExecCode(ctx, *node.Script, inputs)
	returnInfo.Error = errMsg
	returnInfo.ReturnValue = ret
	if err != nil {
		returnInfo.Suc = false
	}

	if err != nil || errMsg != "" {
		job.Status = model.WorkflowJobFailed
	}

	job.ReturnInfo = datatypes.NewJSONType(returnInfo)
	job.UpdatedAt = time.Now()

	if err := d.workflowStore.UpdateData(ctx, job, map[string]any{
		"uuid": job.UUID,
	}, "status", "feedback_data", "return_info", "updated_at"); err != nil {
		logger.Errorf(ctx, "onJobStatus update job fail uuid: %s, err: %+v", job.UUID, err)
	}

	return err
}

func (d *notebookEngine) sendAction(_ context.Context, node *model.WorkflowNode, job *model.WorkflowNodeJob) error {
	if d.session.IsClosed() {
		return code.EdgeConnectClosedErr
	}

	data := lab.EdgeData[*engine.SendActionData]{
		EdgeMsg: lab.EdgeMsg{
			Action: lab.JobStart,
		},
		Data: &engine.SendActionData{
			DeviceID:   *node.DeviceName,
			Action:     node.ActionName,
			ActionType: node.ActionType,
			ActionArgs: node.Param,
			JobID:      job.UUID,
			TaskID:     d.currentTaskUUID,
			NodeID:     node.UUID,
			ServerInfo: engine.ServerInfo{
				SendTimestamp: float64(time.Now().UnixNano()) / 1e9,
			},
		},
	}

	b, err := json.Marshal(data)
	if err != nil {
		return code.NodeDataMarshalErr.WithErr(err)
	}

	return d.session.Write(b)
}

func (d *notebookEngine) callbackAction(ctx context.Context, key engine.ActionKey, job *model.WorkflowNodeJob) error {
	for {
		select {
		case <-d.cancelCtx.Done():
			return code.JobCanceled
		default:
		}

		time.Sleep(time.Millisecond * 500)
		value, exist := d.GetDeviceActionStatus(ctx, key)
		if !exist {
			return code.CallbackJobStatusKeyNotExistErr
		}

		if value.Free {
			d.DelStatus(ctx, key)
			break
		}

		if value.Timestamp.Unix() < time.Now().Unix() {
			return code.JobTimeoutErr
		}
	}

	// 查询任务状态是否回调成功
	err := d.workflowStore.GetData(ctx, job, map[string]any{
		"id": job.ID,
	})
	if err != nil {
		return err
	}

	logger.Infof(ctx, "schedule job run finished: %d", job.ID)
	switch job.Status {
	case model.WorkflowJobSuccess:
		return nil
	case model.WorkflowJobFailed:
		return code.JobRunFailErr
	default:
		return code.JobRunFailErr
	}
}

func (d *notebookEngine) GetStatus(_ context.Context) error {
	return nil
}

func (d *notebookEngine) OnJobUpdate(ctx context.Context, data *engine.JobData) error {
	if data.Status == "running" || data.Status == "pending" {
		return nil
	}

	job, ok := d.jobMap[data.JobID]
	if ok {
		job.ReturnInfo = data.ReturnInfo
		job.FeedbackData = data.FeedbackData
		job.Status = model.WorkflowJobStatus(data.Status)
		d.createSampleRecord(ctx, job.ID, data.ReturnInfo.Data().Samples)
	} else {
		logger.Warnf(ctx, "notebookEngine.OnJobUpdate can not found job uuid: %+v", data.JobID)
	}

	if err := d.workflowStore.UpdateData(ctx, &model.WorkflowNodeJob{
		Status:       model.WorkflowJobStatus(data.Status),
		FeedbackData: data.FeedbackData,
		ReturnInfo:   data.ReturnInfo,
		BaseModel: model.BaseModel{
			UpdatedAt: time.Now(),
		},
		// Timestamp:    data.Timestamp,
	}, map[string]any{
		"uuid": data.JobID,
	}, "status", "feedback_data", "return_info", "updated_at"); err != nil {
		logger.Errorf(ctx, "onJobStatus update job fail uuid: %s, err: %+v", data.JobID, err)
	}

	d.SetDeviceActionStatus(ctx, engine.ActionKey{
		Type:       engine.JobCallbackStatus,
		TaskUUID:   data.TaskID,
		JobID:      data.JobID,
		DeviceID:   data.DeviceID,
		ActionName: data.ActionName,
	}, true, 0)

	return nil
}

func (d *notebookEngine) createSampleRecord(ctx context.Context, jobID int64, samples []*model.SampleValue) {
	if jobID <= 0 {
		return
	}

	sampleUUIDs := utils.FilterUniqSlice(samples, func(s *model.SampleValue) (uuid.UUID, bool) {
		return s.SampleUUID, true
	})
	if len(sampleUUIDs) == 0 {
		return
	}

	sampleUUID2IDMap := d.workflowStore.UUID2ID(ctx, &model.Sample{}, sampleUUIDs...)
	if len(sampleUUID2IDMap) != len(sampleUUIDs) {
		logger.Errorf(ctx, "notebookEngine.createSampleRecord uuid not exist uuids: %+v", sampleUUIDs)
	}

	sampleDatas := utils.FilterSlice(samples, func(s *model.SampleValue) (*model.WorkflowNodeJobSample, bool) {
		return &model.WorkflowNodeJobSample{
			JobID:    jobID,
			SampleID: sampleUUID2IDMap[s.SampleUUID],
			OssPath:  s.OssPath,
			Extra:    s.Extra,
		}, true
	})

	if err := d.workflowStore.CreateSample(ctx, sampleDatas); err != nil {
		logger.Errorf(ctx, "notebookEngine.CreateSample fail err: %+v", err)
	}
}

// func (d *notebookEngine) updateTaskStatus(ctx context.Context, status model.WorkflowTaskStatus, taskID int64) {
// 	data := &model.WorkflowTask{
// 		Status:       status,
// 		FinishedTime: time.Now(),
// 	}
// 	data.UpdatedAt = time.Now()
// 	if err := d.workflowStore.UpdateData(context.Background(), data, map[string]any{
// 		"id": taskID,
// 	}, "status", "updated_at", "finished_time"); err != nil {
// 		logger.Errorf(ctx, "engine dag updateTask id: %d, err: %+v", taskID, err)
// 	}
// }

func (d *notebookEngine) updateJob(ctx context.Context, status model.WorkflowJobStatus, jobID int64) {
	data := &model.WorkflowNodeJob{
		Status: status,
	}
	data.UpdatedAt = time.Now()

	if err := d.workflowStore.UpdateData(context.Background(), data, map[string]any{
		"id": jobID,
	}, "status", "updated_at"); err != nil {
		logger.Errorf(ctx, "engine dag updateJob job id: %+v, err: %+v", jobID, err)
	}
}

func (d *notebookEngine) GetDeviceActionStatus(ctx context.Context, key engine.ActionKey) (engine.ActionValue, bool) {
	valueI, ok := d.actionStatus.Load(key)
	if !ok {
		return engine.ActionValue{}, false
	}
	return valueI.(engine.ActionValue), true
}

func (d *notebookEngine) SetDeviceActionStatus(ctx context.Context, key engine.ActionKey, free bool, needMore time.Duration) {
	valueI, ok := d.actionStatus.Load(key)
	if ok {
		value := valueI.(engine.ActionValue)
		value.Free = free
		value.Timestamp = value.Timestamp.Add(needMore)
		logger.Warnf(ctx, "SetDeviceActionStatus key: %+v, value: %+v, more: %d", key, value, needMore)
		d.actionStatus.Store(key, value)
	} else {
		logger.Warnf(ctx, "SetDeviceActionStatus not found key: %+v", key)
	}
}

func (d *notebookEngine) InitDeviceActionStatus(ctx context.Context, key engine.ActionKey, start time.Time, free bool) {
	d.actionStatus.Store(key, engine.ActionValue{
		Timestamp: start,
		Free:      free,
	})
}

func (d *notebookEngine) DelStatus(ctx context.Context, key engine.ActionKey) {
	d.actionStatus.Delete(key)
}

func (d *notebookEngine) Type(ctx context.Context) engine.JobType {
	return engine.NotebookJobType
}

func (d *notebookEngine) ID(ctx context.Context) uuid.UUID {
	if d.job == nil {
		return uuid.NewNil()
	}

	return d.currentTaskUUID
}

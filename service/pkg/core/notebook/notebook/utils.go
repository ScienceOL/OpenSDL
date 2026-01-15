package notebook

import (
	// 外部依赖
	"errors"
	"sort"

	// 内部引用
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	model "github.com/scienceol/opensdl/service/pkg/model"
)

// SortWorkflowNodesByDAG 按照DAG依赖顺序排序WorkflowNode
func SortWorkflowNodesByDAG(nodes []*model.WorkflowNode, edges []*model.WorkflowEdge) ([]*model.WorkflowNode, error) {
	if len(nodes) == 0 {
		return nodes, nil
	}

	// 创建节点映射：UUID -> WorkflowNode
	nodeMap := make(map[uuid.UUID]*model.WorkflowNode)
	// 创建节点映射：UUID -> 入度（指向该节点的边数）
	inDegree := make(map[uuid.UUID]int)
	// 创建邻接表：源节点UUID -> 目标节点UUID列表
	graph := make(map[uuid.UUID][]uuid.UUID)

	// 初始化节点映射和入度
	for _, node := range nodes {
		nodeMap[node.UUID] = node
		inDegree[node.UUID] = 0
	}

	// 构建图结构
	for _, edge := range edges {
		// 确保边连接的节点存在
		if _, exists := nodeMap[edge.SourceNodeUUID]; !exists {
			continue
		}
		if _, exists := nodeMap[edge.TargetNodeUUID]; !exists {
			continue
		}

		// 添加边到邻接表
		graph[edge.SourceNodeUUID] = append(graph[edge.SourceNodeUUID], edge.TargetNodeUUID)
		// 目标节点入度+1
		inDegree[edge.TargetNodeUUID]++
	}

	// 找到所有入度为0的节点（起始节点）
	queue := make([]uuid.UUID, 0)
	for uuid, degree := range inDegree {
		if degree == 0 {
			queue = append(queue, uuid)
		}
	}

	// 如果没有起始节点，说明存在环
	if len(queue) == 0 {
		return nil, errors.New("图中存在环，无法进行拓扑排序")
	}

	// 对起始节点按照ID排序
	sortQueueByID(queue, nodeMap)

	result := make([]*model.WorkflowNode, 0, len(nodes))
	currentLevel := make([]*model.WorkflowNode, 0)

	// 层级遍历
	for len(queue) > 0 {
		levelSize := len(queue)
		currentLevel = currentLevel[:0] // 清空当前层级

		// 处理当前层级的所有节点
		for i := range levelSize {
			currentUUID := queue[i]
			currentNode := nodeMap[currentUUID]
			currentLevel = append(currentLevel, currentNode)

			// 处理当前节点的所有邻居
			for _, neighborUUID := range graph[currentUUID] {
				inDegree[neighborUUID]--
				// 如果邻居节点的入度降为0，加入下一层队列
				if inDegree[neighborUUID] == 0 {
					queue = append(queue, neighborUUID)
				}
			}
		}

		// 对当前层级节点按照ID排序
		sort.Slice(currentLevel, func(i, j int) bool {
			return currentLevel[i].ID < currentLevel[j].ID
		})

		// 将当前层级节点加入结果
		result = append(result, currentLevel...)

		// 移除已处理的节点，准备处理下一层
		queue = queue[levelSize:]

		// 对下一层队列按照ID排序（保证同一层级内按ID顺序处理）
		sortQueueByID(queue, nodeMap)
	}

	// 检查是否所有节点都被处理
	if len(result) != len(nodes) {
		return nil, code.WorkflowHasCircularErr
	}

	return result, nil
}

// sortQueueByID 按照节点ID对UUID队列进行排序
func sortQueueByID(queue []uuid.UUID, nodeMap map[uuid.UUID]*model.WorkflowNode) {
	sort.Slice(queue, func(i, j int) bool {
		return nodeMap[queue[i]].ID < nodeMap[queue[j]].ID
	})
}

use super::dto::{
	AnchorReq, AnchorRes, CallToolReq, CallToolRes, DegradeReq, DegradeRes, DescriptorReq,
	DescriptorRes, ForgetReq, ForgetRes, HealthRes, IngestReq, IngestRes, LinkReq, LinkRes,
	ListToolsReq, ListToolsRes, NeighborsReq, NeighborsRes, PulseReq, PulseRes, QueryReq, QueryRes,
};

crate::service! {
		pub trait KernRpc {
				async fn query(req: QueryReq) -> QueryRes;
				async fn ingest(req: IngestReq) -> IngestRes;
				async fn link(req: LinkReq) -> LinkRes;
				async fn neighbors(req: NeighborsReq) -> NeighborsRes;
				async fn forget(req: ForgetReq) -> ForgetRes;
				async fn degrade(req: DegradeReq) -> DegradeRes;
				async fn health() -> HealthRes;
				async fn anchor(req: AnchorReq) -> AnchorRes;
				async fn descriptor(req: DescriptorReq) -> DescriptorRes;
				async fn pulse(req: PulseReq) -> PulseRes;
				async fn call_tool(req: CallToolReq) -> CallToolRes;
				async fn list_tools(req: ListToolsReq) -> ListToolsRes;
		}
}
